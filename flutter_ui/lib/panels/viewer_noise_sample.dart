// Denoise's sample, drawn on the picture.
//
// The noise profile is measured inside a square of the shot, and a square is
// something the eye can judge: is it on a flat patch, is it clear of edges,
// is it big enough. With a Denoise effect selected the square is drawn where
// its Sample rows put it, the way the transform gizmo draws a layer's box.
//
// The square is in the layer's own pixels, where Denoise measures, so it is
// drawn through the layer's own map: on a scaled, turned or parented layer it
// is the shape the layer makes of it, and a drag is worked out in those pixels
// too.
//
// With the Selection tool in hand the square moves by its middle and grows by
// its corners, about its centre. Nothing is written while it moves: the rows
// it would change are the sample's own, and a preview of them would show
// nothing. The release measures the noise in the square and writes the
// sample and what it found as one undo step, so the profile follows the
// square, as it does in other software's noise tools.

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/denoise.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart' show fireEffectAction;
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart' show sampledExpression, timeOfFrame;
import '../state/dock.dart' show Panel;
import '../widgets/controls.dart' show ThemeScope;
import 'effect_param_row_frb.dart' show scalarValueAt;
import 'layer_fold_frb.dart' show effectPath;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_gizmo.dart' show LayerBox, gizmoHandleSize;
import 'viewer_layer_map.dart' show ViewerLayerMap;
import 'viewer_menus.dart' show viewerRow;

/// The effect the square belongs to, by its match name.
const String denoiseMatchName = 'denoise';

/// The rows the square reads and a release writes.
const String sampleX = 'sample_x', sampleY = 'sample_y', sampleSize = 'sample_size';

/// The smallest and largest square, in the layer's own pixels: the Sample size
/// row's own range.
const double _minSide = 16, _maxSide = 512;

/// How far round a corner a press still takes the corner, in screen pixels.
const double _cornerSlop = 12;

/// One Denoise's square on screen.
class _Square {
  final LayerReference layer;
  final UuidValue effect;

  /// Centre and side in the layer's own pixels.
  final Offset centre;
  final double side;

  /// The layer's own picture, in its own pixels, and where the layer puts
  /// those pixels on screen.
  final Size size;
  final ViewerLayerMap map;

  /// An expression or a wire works out one of the three rows, so the square
  /// stays where it is.
  final bool fixed;

  const _Square(this.layer, this.effect, this.centre, this.side, this.size,
      this.map, this.fixed);

  String get key => '${layer.internallayerId}/$effect';
}

/// What a press took hold of.
enum _Grip { move, topLeft, topRight, bottomLeft, bottomRight }

class ViewerNoiseSample extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Every layer's box as the gizmo has it, asked for only once a square is
  /// to be drawn.
  final List<LayerBox> Function() boxes;

  /// The armed tool. Only Selection moves the square.
  final ToolMode tool;

  /// Told once a release has been written.
  final VoidCallback onChanged;

  const ViewerNoiseSample({
    super.key,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.tool,
    required this.onChanged,
  });

  @override
  State<ViewerNoiseSample> createState() => _ViewerNoiseSampleState();
}

class _ViewerNoiseSampleState extends State<ViewerNoiseSample> {
  /// The square in hand, what took it, the corner of its patch on screen and
  /// the press within it, and the square as the drag has it now, in the
  /// layer's own pixels.
  _Square? _held;
  _Grip _grip = _Grip.move;
  Offset _patch = Offset.zero;
  Offset _down = Offset.zero;
  Offset? _centre;
  double? _side;

  /// A release being measured: the square stays where it was dropped until
  /// the write lands, rather than jumping back for a moment.
  final Map<String, (Offset, double)> _landing = {};

  String? _hover;
  _Grip? _hoverGrip;

  LumitUiState get _ui => widget.uiState;

  @override
  Widget build(BuildContext context) => Positioned.fill(
        child: ListenableBuilder(
          listenable: Listenable.merge([
            _ui.selectedEffects,
            _ui.selectedProperties,
            _ui.model,
            _ui.playheadFrame,
            // A layer moved in the Viewer or scrubbed in its rows carries its
            // square with it.
            _ui.liveTransforms,
          ]),
          builder: (context, _) {
            final squares = _squares();
            // A square that went away under a drag (the selection moved, the
            // effect was switched off) takes the drag with it: its detector
            // left the tree, so no end or cancel ever arrives.
            if (_held != null && !squares.any((s) => s.key == _held!.key)) {
              _held = null;
              _centre = null;
              _side = null;
            }
            if (squares.isEmpty) return const SizedBox.shrink();
            final t = ThemeScope.of(context).theme;
            return Stack(
              children: [
                Positioned.fill(
                  child: IgnorePointer(
                    child: CustomPaint(
                      painter: _SquarePainter(
                        quads: [for (final s in squares) _onScreen(s)],
                        lit: [
                          for (final s in squares)
                            s.key == _held?.key || s.key == _hover
                        ],
                        accent: t.accent,
                        surface: t.surface0,
                      ),
                    ),
                  ),
                ),
                if (widget.tool.group == ToolGroup.select)
                  for (final s in squares) _grips(s),
              ],
            );
          },
        ),
      );

  /// Where [s] is drawn: the drag's square while it is in hand, the dropped
  /// one while its measure is landing, the rows' otherwise.
  List<Offset> _onScreen(_Square s) {
    if (s.key == _held?.key && _centre != null && _side != null) {
      return _corners(s.map, _centre!, _side!);
    }
    final landing = _landing[s.key];
    return landing == null
        ? _corners(s.map, s.centre, s.side)
        : _corners(s.map, landing.$1, landing.$2);
  }

  /// The square's corners on screen: top left, top right, bottom right and
  /// bottom left, as the layer's own picture has them.
  static List<Offset> _corners(ViewerLayerMap map, Offset centre, double side) {
    final h = side / 2;
    return [
      map.toScreen(centre.dx - h, centre.dy - h),
      map.toScreen(centre.dx + h, centre.dy - h),
      map.toScreen(centre.dx + h, centre.dy + h),
      map.toScreen(centre.dx - h, centre.dy + h),
    ];
  }

  /// The square's middle and its four corners, each a patch that takes the
  /// pointer. The middle is the whole square less the corners.
  Widget _grips(_Square s) {
    final quad = _onScreen(s);
    Widget patch(_Grip grip, Rect at, MouseCursor cursor, {Path? shape}) =>
        Positioned.fromRect(
          key: ValueKey<String>('viewer-noise-sample-${s.key}-${grip.name}'),
          rect: at,
          child: _Shaped(
            shape: shape,
            child: MouseRegion(
            cursor: s.fixed ? MouseCursor.defer : cursor,
            onEnter: (_) => setState(() {
              _hover = s.key;
              _hoverGrip = grip;
            }),
            onExit: (_) => setState(() {
              if (_hoverGrip == grip) {
                _hover = null;
                _hoverGrip = null;
              }
            }),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapUp: (d) => _menu(s, d.globalPosition),
                onPanDown: (d) => _start(s, grip, at.topLeft, d.localPosition),
                onPanUpdate: (d) => _move(d.localPosition),
                onPanEnd: (_) => _end(),
                onPanCancel: _cancel,
              ),
            ),
          ),
        );
    // A corner reaches no further than a quarter of the square's shorter
    // side, so a square that is small on screen still has a middle to hold.
    final shorter = (quad[1] - quad[0]).distance < (quad[2] - quad[1]).distance
        ? (quad[1] - quad[0]).distance
        : (quad[2] - quad[1]).distance;
    final reach = (shorter / 4).clamp(0.0, _cornerSlop);
    Rect corner(Offset at) =>
        Rect.fromCenter(center: at, width: reach * 2, height: reach * 2);
    // The middle takes the pointer inside the square only, which on a turned
    // layer is not the rectangle round it.
    final around = Rect.fromPoints(quad[0], quad[2])
        .expandToInclude(Rect.fromPoints(quad[1], quad[3]));
    final inside = Path()
      ..addPolygon([for (final c in quad) c - around.topLeft], true);
    return Stack(children: [
      patch(_Grip.move, around, SystemMouseCursors.move, shape: inside),
      patch(_Grip.topLeft, corner(quad[0]),
          SystemMouseCursors.resizeUpLeftDownRight),
      patch(_Grip.bottomRight, corner(quad[2]),
          SystemMouseCursors.resizeUpLeftDownRight),
      patch(_Grip.topRight, corner(quad[1]),
          SystemMouseCursors.resizeUpRightDownLeft),
      patch(_Grip.bottomLeft, corner(quad[3]),
          SystemMouseCursors.resizeUpRightDownLeft),
    ]);
  }

  void _menu(_Square s, Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (closeWith) {
        void close() => closeWith(null);
        return [
          viewerRow(engineLabel('Learn'),
              () => learnNoise(s.layer, s.effect, _ui), close,
              id: 'viewer-noise-sample-learn'),
          viewerRow(l10n.viewerMenuPointReveal, () {
            _ui.setEffectSelection(s.layer, [s.effect]);
            _ui.frontPanel(Panel.effectControls);
          }, close, id: 'viewer-noise-sample-reveal'),
        ];
      },
    );
  }

  void _start(_Square s, _Grip grip, Offset patch, Offset at) {
    _cancel();
    if (s.fixed) return;
    setState(() {
      _held = s;
      _grip = grip;
      _patch = patch;
      _down = at;
      _centre = s.centre;
      _side = s.side;
    });
  }

  /// The pointer's travel taken back through the layer's map into its own
  /// pixels. A corner grows the square about its centre, so the side follows
  /// the corner's distance from the middle.
  void _move(Offset at) {
    final s = _held;
    if (s == null) return;
    final d = s.map.layerOf(_patch + at) - s.map.layerOf(_patch + _down);
    setState(() {
      if (_grip == _Grip.move) {
        _centre = Offset(
          (s.centre.dx + d.dx).clamp(0.0, s.size.width),
          (s.centre.dy + d.dy).clamp(0.0, s.size.height),
        );
      } else {
        final sign = switch (_grip) {
          _Grip.topLeft => const Offset(-1, -1),
          _Grip.topRight => const Offset(1, -1),
          _Grip.bottomLeft => const Offset(-1, 1),
          _ => const Offset(1, 1),
        };
        final grown = (d.dx * sign.dx + d.dy * sign.dy);
        _side = (s.side + grown).clamp(_minSide, _maxSide);
      }
    });
  }

  Future<void> _end() async {
    final s = _held;
    final centre = _centre, side = _side;
    setState(() {
      _held = null;
      _centre = null;
      _side = null;
    });
    if (s == null || centre == null || side == null) return;
    if (centre == s.centre && side == s.side) return;
    setState(() => _landing[s.key] = (centre, side));
    await measureNoiseAt(s.layer, s.effect, _ui,
        centre: centre, side: side);
    if (!mounted) return;
    setState(() => _landing.remove(s.key));
    widget.onChanged();
  }

  void _cancel() {
    if (_held == null) return;
    setState(() {
      _held = null;
      _centre = null;
      _side = null;
    });
  }

  List<_Square> _squares() {
    final picked = _ui.selectedEffects.value;
    final rows = _ui.selectedProperties.value;
    if (picked.isEmpty && rows.isEmpty) return const [];
    final owner = _ui.selectedEffectsLayer?.internallayerId;
    final frame = _ui.playheadFrame.value;
    late final boxes = {for (final b in widget.boxes()) b.id: b};
    final out = <_Square>[];
    for (final entry in _ui.model.layers) {
      final layerId = entry.layer.internallayerId;
      for (final fx in entry.info.effects) {
        if (fx.name != denoiseMatchName || !fx.enabled) continue;
        final path = effectPath('$layerId', '${fx.id}');
        final whole = (layerId == owner && picked.contains(fx.id)) ||
            rows.contains(path) ||
            rows.contains('$path/$sampleX') ||
            rows.contains('$path/$sampleY') ||
            rows.contains('$path/$sampleSize');
        if (!whole) continue;
        // A layer that is not on the picture has nowhere to put a square.
        final box = boxes[layerId];
        if (box == null || box.bounds.isEmpty) continue;
        BridgeScalar? held(String id) =>
            switch (fx.values.where((v) => v.id == id).firstOrNull?.value) {
              BridgeEffectValue_Float(:final field0) => field0,
              _ => null,
            };
        double? at(String id) {
          final s = held(id);
          return s is BridgeScalar_Expression
              ? sampledExpression(s, timeOfFrame(widget.comp, frame),
                  entry.layer, _ui.model.heldRevision,
                  row: BridgeExpressionRow.effect(effect: fx.id, param: id))
              : scalarValueAt(s, widget.comp, frame);
        }

        final x = at(sampleX), y = at(sampleY), side = at(sampleSize);
        if (x == null || y == null || side == null) continue;
        final centre = Offset(x, y);
        // Behind the camera, a square has no place on the picture.
        final h = side / 2;
        if (![(-h, -h), (h, -h), (h, h), (-h, h)]
            .every((c) => box.map.inFront(x + c.$1, y + c.$2))) {
          continue;
        }
        out.add(_Square(
          entry.layer,
          fx.id,
          centre,
          side,
          box.bounds,
          box.map,
          [sampleX, sampleY, sampleSize]
              .any((id) => held(id) is BridgeScalar_Expression),
        ));
      }
    }
    return out;
  }
}

/// Rung whenever a measure of the noise starts or lands, from a button, a key,
/// a menu or the square, so a display reads the engine's status again however
/// the measure was asked for.
final ValueNotifier<int> noiseMeasures = ValueNotifier<int>(0);

/// Learn at the playhead: find the flattest patch, put the square on it and
/// measure it. The display beside the rows hears the result.
void learnNoise(LayerReference layer, UuidValue effect, LumitUiState ui) {
  try {
    fireEffectAction(
        layer: layer,
        effect: effect,
        param: 'learn',
        frame: ui.playheadFrame.value);
  } catch (_) {
    // The display reads why from the status; a refusal here has nothing
    // more to say.
  }
  noiseMeasures.value++;
}

/// Measure in a square the hand put down, and write it with what it found.
Future<void> measureNoiseAt(
    LayerReference layer, UuidValue effect, LumitUiState ui,
    {required Offset centre, required double side}) async {
  try {
    await denoiseMeasureAt(
        layer: layer,
        effect: effect,
        frame: ui.playheadFrame.value,
        centreX: centre.dx,
        centreY: centre.dy,
        side: side);
  } catch (_) {
    // Nothing to measure with (no graphics adapter, a layer with no
    // picture): the status says so on the display.
  }
  noiseMeasures.value++;
}

/// The squares: a hairline in the accent with a handle at each corner, the
/// transform gizmo's own figure, so the two read as one family.
class _SquarePainter extends CustomPainter {
  /// Each square's corners on screen, in order round it.
  final List<List<Offset>> quads;
  final List<bool> lit;
  final Color accent, surface;

  const _SquarePainter({
    required this.quads,
    required this.lit,
    required this.accent,
    required this.surface,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final edge = Paint()
      ..color = accent
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke;
    for (var i = 0; i < quads.length; i++) {
      final quad = quads[i];
      final outline = Path()..addPolygon(quad, true);
      // A dark line either side of the accent one, so the square reads on a
      // white sky as well as on a black street.
      canvas.drawPath(
          outline,
          Paint()
            ..color = surface.withValues(alpha: 0.6)
            ..strokeWidth = 3
            ..style = PaintingStyle.stroke);
      canvas.drawPath(outline, lit[i] ? (edge..strokeWidth = 1.5) : edge);
      edge.strokeWidth = 1;
      for (final corner in quad) {
        final handle = Rect.fromCenter(
            center: corner, width: gizmoHandleSize, height: gizmoHandleSize);
        canvas.drawRect(handle, Paint()..color = surface);
        canvas.drawRect(handle, edge);
      }
    }
  }

  @override
  bool shouldRepaint(_SquarePainter old) =>
      old.accent != accent ||
      old.surface != surface ||
      old.quads.length != quads.length ||
      [for (var i = 0; i < quads.length; i++) i]
          .any((i) => !listEquals(old.quads[i], quads[i])) ||
      !listEquals(old.lit, lit);
}

/// [child], taking the pointer only inside [shape] when there is one.
class _Shaped extends StatelessWidget {
  final Path? shape;
  final Widget child;

  const _Shaped({required this.shape, required this.child});

  @override
  Widget build(BuildContext context) {
    final at = shape;
    return at == null
        ? child
        : ClipPath(clipper: _PathClip(at), child: child);
  }
}

class _PathClip extends CustomClipper<Path> {
  final Path path;
  const _PathClip(this.path);

  @override
  Path getClip(Size size) => path;

  @override
  bool shouldReclip(_PathClip old) => old.path != path;
}
