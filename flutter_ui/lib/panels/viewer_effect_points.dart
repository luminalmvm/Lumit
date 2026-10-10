// The points of the selected effects, marked on the picture.
//
// A Centre or a Light is an x and a y in the panel, and nothing on the picture
// said where that was. With the effect selected, or either of the pair's rows
// in the Timeline, each point wears the anchor's mark at the spot a pick there
// would write.

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../state/comp_time.dart';
import 'effect_param_row_frb.dart'
    show EffectStackEditor, cachedListPairs, cachedListParameters;
import 'layer_fold_frb.dart' show effectPath;
import 'viewer_tool_cursor.dart' show paintAnchorMark;

class ViewerEffectPoints extends StatelessWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Where the picture is drawn in this panel, at the current magnification.
  final Rect fitted;
  final Size compSize;
  final Color colour;

  const ViewerEffectPoints({
    super.key,
    required this.comp,
    required this.uiState,
    required this.fitted,
    required this.compSize,
    required this.colour,
  });

  @override
  Widget build(BuildContext context) => Positioned.fill(
        child: IgnorePointer(
          child: ListenableBuilder(
            listenable: Listenable.merge([
              uiState.selectedEffects,
              uiState.selectedProperties,
              uiState.model,
              uiState.playheadFrame,
              EffectStackEditor.staging,
            ]),
            builder: (context, _) {
              final points = _points();
              if (points.isEmpty) return const SizedBox.shrink();
              return CustomPaint(painter: _PointsPainter(points, colour));
            },
          ),
        ),
      );

  List<Offset> _points() {
    final picked = uiState.selectedEffects.value;
    final rows = uiState.selectedProperties.value;
    if (picked.isEmpty && rows.isEmpty) return const [];
    final owner = uiState.selectedEffectsLayer?.internallayerId;
    final staged = EffectStackEditor.staging.value;
    final frame = uiState.playheadFrame.value;
    final out = <Offset>[];
    for (final entry in uiState.model.layers) {
      final layerId = entry.layer.internallayerId;
      for (final fx in entry.info.effects) {
        final path = effectPath('$layerId', '${fx.id}');
        final whole = (layerId == owner && picked.contains(fx.id)) ||
            rows.contains(path);
        final params = cachedListParameters(fx.name);
        for (final pair in cachedListPairs(fx.name)) {
          if (!whole &&
              !rows.contains('$path/${pair.x}') &&
              !rows.contains('$path/${pair.y}')) {
            continue;
          }
          // The pick's own rule, so the mark sits where a pick would put it:
          // px is comp pixels, per cent is of the frame, and nothing else is
          // a position.
          final unit = params.where((p) => p.id == pair.x).firstOrNull?.unit;
          final span = switch (unit) {
            BridgeUnit.px => compSize,
            BridgeUnit.percent => const Size(100, 100),
            _ => Size.zero,
          };
          if (span.isEmpty) continue;
          // What a drag has staged first, so the mark follows the picture.
          double? at(String id) => switch (staged[(fx.id, id)] ??
              fx.values.where((v) => v.id == id).firstOrNull?.value) {
                BridgeEffectValue_Float(
                  field0: BridgeScalar_Static(:final field0)
                ) =>
                  field0,
                BridgeEffectValue_Float(
                  field0: final BridgeScalar_Keyframed keyed
                ) =>
                  sampledScalar(keyed, timeOfFrame(comp, frame)),
                _ => null,
              };
          final x = at(pair.x), y = at(pair.y);
          if (x == null || y == null) continue;
          out.add(Offset(
            fitted.left + x / span.width * fitted.width,
            fitted.top + y / span.height * fitted.height,
          ));
        }
      }
    }
    return out;
  }
}

class _PointsPainter extends CustomPainter {
  final List<Offset> points;
  final Color colour;

  const _PointsPainter(this.points, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    for (final at in points) {
      paintAnchorMark(canvas, at, colour);
    }
  }

  @override
  bool shouldRepaint(_PointsPainter old) =>
      old.colour != colour || !listEquals(old.points, points);
}
