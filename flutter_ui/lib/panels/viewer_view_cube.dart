// The axes in a 3D viewport's corner: which way x, y and z point as the view
// sees them, a click on an end to look straight along it, as Top, Front,
// Left and the rest, and a drag on them to orbit. Under them, the switch
// between perspective and orthographic, and the one that draws the view as
// lines only.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/viewer_view.dart' show ViewerView;
import '../widgets/controls.dart';
import 'viewer_camera.dart' show CameraPose;
import 'viewer_gizmo_3d.dart' show gizmoAxisColours;

/// How far in from the viewport's top right corner the axes sit.
const double viewCubeInset = 26;

/// The axes' reach from their middle, and the size of the ends.
const double _arm = 24;
const double _end = 6.5;
const double _side = 2 * (_arm + _end + 2);

/// The view each end looks from: the + and - ends of x, y and z in turn.
/// y runs down the comp, so its - end is Top.
const List<ViewerView> _faces = [
  ViewerView.right,
  ViewerView.left,
  ViewerView.bottom,
  ViewerView.top,
  ViewerView.back,
  ViewerView.front,
];

class ViewerViewCube extends StatefulWidget {
  /// The view's rotation, degrees, as its pose holds it.
  final (double, double, double) rotation;
  final bool ortho;
  final bool wireframe;

  /// Look along an end: one of the six fixed views' directions.
  final ValueChanged<ViewerView> onSnap;

  /// A drag on the axes, which orbits.
  final VoidCallback onOrbitStart;
  final ValueChanged<Offset> onOrbit;
  final VoidCallback onOrbitEnd;
  final VoidCallback onOrtho;
  final VoidCallback onWireframe;

  const ViewerViewCube({
    super.key,
    required this.rotation,
    required this.ortho,
    required this.wireframe,
    required this.onSnap,
    required this.onOrbitStart,
    required this.onOrbit,
    required this.onOrbitEnd,
    required this.onOrtho,
    required this.onWireframe,
  });

  @override
  State<ViewerViewCube> createState() => _ViewerViewCubeState();
}

class _ViewerViewCubeState extends State<ViewerViewCube> {
  int? _hover;

  /// Each end's place from the middle, and how far it is from the eye: the
  /// world's axes taken through the view's own three.
  List<(Offset, double)> get _ends {
    final a =
        CameraPose(position: (0, 0, 0), rotation: widget.rotation, zoom: 1)
            .axes;
    final out = <(Offset, double)>[];
    for (var k = 0; k < 3; k++) {
      double pick((double, double, double) v) => [v.$1, v.$2, v.$3][k];
      final at = Offset(pick(a.right), pick(a.up)) * _arm;
      final depth = pick(a.forward);
      out
        ..add((at, depth))
        ..add((-at, -depth));
    }
    return out;
  }

  int? _endAt(Offset local) {
    final centre = const Offset(_side / 2, _side / 2);
    final ends = _ends;
    int? best;
    var nearest = _end + 3;
    // Where two ends overlap the nearer one wins, as it is drawn on top: a
    // far end counts as up to 2 px further off.
    for (var i = 0; i < ends.length; i++) {
      final d = (local - (centre + ends[i].$1)).distance + ends[i].$2 * 2;
      if (d < nearest) {
        nearest = d;
        best = i;
      }
    }
    return best;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final hover = _hover;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.end,
      mainAxisSize: MainAxisSize.min,
      children: [
        MouseRegion(
          key: const ValueKey('viewport-axes'),
          cursor: hover == null ? MouseCursor.defer : SystemMouseCursors.click,
          onHover: (e) {
            final at = _endAt(e.localPosition);
            if (at != _hover) setState(() => _hover = at);
          },
          onExit: (_) => setState(() => _hover = null),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: (d) {
              final at = _endAt(d.localPosition);
              if (at != null) widget.onSnap(_faces[at]);
            },
            onPanStart: (_) => widget.onOrbitStart(),
            onPanUpdate: (d) => widget.onOrbit(d.delta),
            onPanEnd: (_) => widget.onOrbitEnd(),
            onPanCancel: widget.onOrbitEnd,
            child: CustomPaint(
              size: const Size(_side, _side),
              painter: _AxesPainter(
                ends: _ends,
                hover: hover,
                plate: t.surface0.withValues(alpha: 0.6),
                ring: t.textPrimary,
                letters: [
                  l10n.viewportAxisX,
                  l10n.viewportAxisY,
                  l10n.viewportAxisZ
                ],
                letterStyle: t.caption.copyWith(color: t.surface0),
              ),
            ),
          ),
        ),
        // What the end under the pointer looks from, by its name.
        SizedBox(
          height: 14,
          child: hover == null
              ? null
              : Text(_faces[hover].title, style: t.caption),
        ),
        // The switches sit on a plate like the axes', so they read over any
        // picture.
        DecoratedBox(
          decoration: BoxDecoration(
            color: t.surface0.withValues(alpha: 0.75),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              LumitTooltip(
                message: l10n.tipViewportWireframe,
                child: HouseButton(
                  key: const ValueKey('viewport-wireframe'),
                  frameless: true,
                  small: true,
                  active: widget.wireframe,
                  padding: const EdgeInsets.all(3),
                  onPressed: widget.onWireframe,
                  child: lumitIcon(
                    LumitIcon.wireframe,
                    size: 14,
                    color: widget.wireframe ? t.accent : t.textSecondary,
                  ),
                ),
              ),
              LumitTooltip(
                message: l10n.tipViewportProjection,
                child: HouseButton(
                  key: const ValueKey('viewport-ortho'),
                  frameless: true,
                  small: true,
                  active: widget.ortho,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
                  onPressed: widget.onOrtho,
                  child: Text(
                    widget.ortho ? l10n.viewportOrtho : l10n.viewportPersp,
                    style: t.caption.copyWith(
                        color: widget.ortho ? t.accent : t.textSecondary),
                  ),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _AxesPainter extends CustomPainter {
  final List<(Offset, double)> ends;
  final int? hover;
  final Color plate;
  final Color ring;
  final List<String> letters;
  final TextStyle letterStyle;

  const _AxesPainter({
    required this.ends,
    required this.hover,
    required this.plate,
    required this.ring,
    required this.letters,
    required this.letterStyle,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final centre = size.center(Offset.zero);
    canvas.drawCircle(centre, _arm + _end + 1, Paint()..color = plate);
    // The far ends first, so the near ones read on top. A + end is a filled
    // disc on its axis with the axis's letter in it, a - end a ring.
    final order = List.generate(ends.length, (i) => i)
      ..sort((a, b) => ends[b].$2.compareTo(ends[a].$2));
    for (final i in order) {
      final (at, depth) = ends[i];
      final colour =
          gizmoAxisColours[i ~/ 2].withValues(alpha: depth > 0.2 ? 0.55 : 1.0);
      final end = centre + at;
      final positive = i.isEven;
      if (positive) {
        canvas.drawLine(
            centre,
            end,
            Paint()
              ..color = colour
              ..strokeWidth = 2);
      }
      canvas.drawCircle(
        end,
        positive ? _end : _end - 1.5,
        Paint()
          ..color = colour
          ..style = positive ? PaintingStyle.fill : PaintingStyle.stroke
          ..strokeWidth = 1.5,
      );
      if (positive) {
        final text = TextPainter(
          text: TextSpan(text: letters[i ~/ 2], style: letterStyle),
          textDirection: TextDirection.ltr,
        )..layout();
        text.paint(canvas, end - Offset(text.width / 2, text.height / 2));
      }
      if (i == hover) {
        canvas.drawCircle(
          end,
          _end + 2,
          Paint()
            ..color = ring
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1,
        );
      }
    }
  }

  @override
  bool shouldRepaint(_AxesPainter old) =>
      old.hover != hover ||
      old.plate != plate ||
      old.ring != ring ||
      old.letterStyle != letterStyle ||
      !_same(old.ends, ends);

  static bool _same(List<(Offset, double)> a, List<(Offset, double)> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}
