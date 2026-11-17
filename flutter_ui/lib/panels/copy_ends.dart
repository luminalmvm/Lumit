// SPDX-License-Identifier: GPL-3.0-only

import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/template/copy.dart';

/// How strongly a copy's intro and outro are shaded on its bar.
const double copyEndsAlpha = 0.28;

/// Shade the stretches of `box` a copy of a graphics template plays its intro
/// and its outro in, `pxPerSecond` being how wide a second is. A copy with
/// neither, or too short to show them apart, draws nothing.
void paintCopyEnds(Canvas canvas, Rect box, BridgeCopy? copy,
    {required double pxPerSecond, required Color ink}) {
  if (copy == null || box.width <= 0) return;
  final intro = copy.introSeconds * pxPerSecond;
  final outro = copy.outroSeconds * pxPerSecond;
  if (intro <= 0 && outro <= 0) return;
  // Both fit the bar, each shrunk by the same amount where they would meet.
  final over = intro + outro - box.width;
  final scale = over > 0 ? box.width / (intro + outro) : 1.0;
  final paint = Paint()..color = ink.withValues(alpha: copyEndsAlpha);
  canvas.save();
  canvas.clipRect(box);
  if (intro > 0) {
    canvas.drawRect(
        Rect.fromLTWH(box.left, box.top, max(intro * scale, 1), box.height),
        paint);
  }
  if (outro > 0) {
    final w = max(outro * scale, 1.0);
    canvas.drawRect(
        Rect.fromLTWH(box.right - w, box.top, w, box.height), paint);
  }
  canvas.restore();
}

/// [paintCopyEnds] on a widget's whole size.
class CopyEndsPainter extends CustomPainter {
  final BridgeCopy? copy;
  final double pxPerSecond;
  final Color ink;

  const CopyEndsPainter(
      {required this.copy, required this.pxPerSecond, required this.ink});

  @override
  void paint(Canvas canvas, Size size) => paintCopyEnds(
      canvas, Offset.zero & size, copy,
      pxPerSecond: pxPerSecond, ink: ink);

  @override
  bool shouldRepaint(CopyEndsPainter old) =>
      old.copy != copy || old.pxPerSecond != pxPerSecond || old.ink != ink;
}
