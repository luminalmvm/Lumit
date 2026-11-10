// A caption layer's captions in the Timeline.
//
// Double-click a caption layer and a row opens under its bar, the way a
// Sequence layer opens its clips: one block per caption, where it shows in
// the comp, with its words inside. Drag an end to retime one and the middle to
// move it. Shut, the bar carries faint marks where the captions are.
//
// The captions come from the comp's [CaptionTracks], read once per change of
// their layer. Drawing walks only the ones in view, found by a binary search
// on their starts.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/caption_tracks.dart';

import '../theme/theme.dart';
import 'timeline_extras_frb.dart';

/// What opening a caption layer adds under its bar: two rows of the table,
/// room for a caption's two lines.
double captionRowHeight(LumitTheme t) => t.density.laneRow * 2;

/// The caption layer whose captions were last picked in the Timeline, so
/// Delete takes those captions rather than the layer.
final ValueNotifier<LayerReference?> captionRowPick = ValueNotifier(null);

/// Ask the Captions panel to open the caption under the playhead on [layer]
/// for typing, or the next one when none is showing. False when [layer] is
/// not a caption layer; with [ask] off, only says whether it is one.
bool typeCaptionAtPlayhead(LumitUiState ui, LayerReference layer,
    {bool ask = true}) {
  final track = ui.captionTracks.track(layer.internallayerId.toString());
  if (track == null) return false;
  if (!ask || track.length == 0) return true;
  final frame = ui.playheadFrame.value.toDouble();
  final i = track.at(frame) ?? math.min(track.before(frame), track.length - 1);
  ui.captionSelection.requestTyping(layer.internallayerId, track.cues[i].index);
  return true;
}

/// Faint marks on a shut caption layer's bar: the stretches no caption
/// covers are shaded, as a Sequence bar shades its gaps, and a hairline
/// stands where one caption runs into the next.
class CaptionMarksPainter extends CustomPainter {
  final CaptionTrack track;
  final TimelineAxis axis;

  /// The bar's own left edge, in the pixels [axis] speaks.
  final double left;
  final Color ink;

  const CaptionMarksPainter({
    required this.track,
    required this.axis,
    required this.left,
    required this.ink,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final gap = Paint()..color = ink.withValues(alpha: 0.45);
    final seam = Paint()..color = ink.withValues(alpha: 0.55);
    var x = 0.0;
    var before = double.negativeInfinity;
    for (var i = 0; i < track.length; i++) {
      final start = axis.xOf(track.starts[i]) - left;
      final end = axis.xOf(track.ends[i]) - left;
      if (end < 0) {
        x = math.max(x, end);
        continue;
      }
      if (start > size.width) break;
      if (start > x + 0.5) {
        canvas.drawRect(Rect.fromLTRB(x, 0, start, size.height), gap);
      } else if (start - before >= captionSeamRoom) {
        // Captions that touch: a hairline between them, while each is wide
        // enough for one to read as a mark rather than as stripes.
        canvas.drawRect(Rect.fromLTWH(start, 0, 1, size.height), seam);
      }
      before = start;
      x = math.max(x, end);
    }
    if (x < size.width) {
      canvas.drawRect(Rect.fromLTRB(x, 0, size.width, size.height), gap);
    }
  }

  @override
  bool shouldRepaint(CaptionMarksPainter old) =>
      old.track != track ||
      old.left != left ||
      old.ink != ink ||
      old.axis.perFrame != axis.perFrame;

  @override
  bool? hitTest(Offset position) => false;
}

/// How wide a caption must be for the seam at its start to be drawn.
const double captionSeamRoom = 6;
