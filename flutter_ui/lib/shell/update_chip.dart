// The update button on the bottom strip, beside the profile.
//
// One small mark that says where the updater has got to, and does the next
// thing when it is pressed: look, download, restart. It draws what
// [UpdateService] already holds and asks it nothing.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/updates.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'menu_bar_frb.dart' show saveProjectFrb;
import 'profile_chip.dart' show SyncRing;
import 'update_dialog_frb.dart';

/// The button on the bottom strip.
class UpdateChip extends StatelessWidget {
  const UpdateChip({super.key});

  @override
  Widget build(BuildContext context) {
    final ui = context.read<LumitUiState>();
    final app = context.read<LumitState>();
    final updates = ui.updates;
    return ListenableBuilder(
      listenable: updates,
      builder: (context, _) => LumitTooltip(
        // The Help menu's row says the same thing in words.
        message: updates.menuLabel,
        maxWidth: 380,
        body: _changes(ThemeScope.of(context).theme, updates),
        child: HouseButton(
          key: const ValueKey('status-update'),
          frameless: true,
          small: true,
          padding: const EdgeInsets.symmetric(horizontal: 4),
          onPressed: updates.busy
              ? null
              : () => pressUpdateButton(
                    context,
                    updates: updates,
                    notice: app.postNotice,
                    projectIsDirty: () => app.project?.isDirty() ?? false,
                    saveProject: () =>
                        saveProjectFrb(app, ui, leaving: true),
                  ),
          child: UpdateMark(updates),
        ),
      ),
    );
  }
}

/// What the waiting update brings, for under the tip: a heading per build
/// and a line per change, newest first, scrolling once it is taller than the
/// tip. Null when there is nothing to list.
Widget? _changes(LumitTheme t, UpdateService updates) {
  // Somebody many releases behind needs the gist, not every line since.
  var room = 60;
  var more = 0;
  final rows = <Widget>[];
  for (final build in updates.builds) {
    final shown = build.changes.take(room).toList();
    room -= shown.length;
    more += build.changes.length - shown.length;
    if (shown.isEmpty) continue;
    rows.add(Padding(
      padding: const EdgeInsets.only(top: 8, bottom: 2),
      child: Text(build.version, style: t.bodyStrong),
    ));
    for (final change in shown) {
      rows.add(Row(
        children: [
          SizedBox(width: 10, child: Text('•', style: t.body)),
          Expanded(
            child: Text(change,
                style: t.bodyPrimary,
                maxLines: 1,
                overflow: TextOverflow.ellipsis),
          ),
        ],
      ));
    }
  }
  if (rows.isEmpty) return null;
  return ConstrainedBox(
    constraints: const BoxConstraints(maxHeight: 320),
    child: SingleChildScrollView(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          ...rows,
          if (more > 0)
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Text(l10n.updateMoreChanges(more), style: t.small),
            ),
        ],
      ),
    ),
  );
}

/// The mark for where [updates] has got to. Quiet while there is nothing to
/// do, and lit once there is an update to fetch or to finish.
class UpdateMark extends StatelessWidget {
  final UpdateService updates;
  const UpdateMark(this.updates, {super.key});

  static const double _size = 16;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox.square(
      dimension: _size,
      child: switch (updates.stage) {
        UpdateStage.checking =>
          const Padding(padding: EdgeInsets.all(1), child: SyncRing()),
        UpdateStage.available => Stack(
            clipBehavior: Clip.none,
            children: [
              glyph.LumitIcon(LumitIcons.downloadUpdate,
                  size: _size, colour: t.textPrimary),
              // The dot that says something is waiting, edged in the strip's
              // own surface so it stands off the arrow.
              Positioned(
                top: -2,
                right: -3,
                child: Container(
                  width: 7,
                  height: 7,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: t.accent,
                    border: Border.all(color: t.surface1),
                  ),
                ),
              ),
            ],
          ),
        UpdateStage.downloading => CustomPaint(
            painter: _DownloadRing(
              updates.progress,
              track: t.hairlineStrong,
              arc: t.accent,
              arrow: t.textPrimary,
            ),
          ),
        UpdateStage.ready => glyph.LumitIcon(LumitIcons.restartToUpdate,
            size: _size, colour: t.accent),
        _ => glyph.LumitIcon(LumitIcons.checkForUpdates,
            size: _size, colour: t.textMuted),
      },
    );
  }
}

/// The download while it comes: the arrow, inside a ring that fills.
class _DownloadRing extends CustomPainter {
  final double progress;
  final Color track, arc, arrow;

  const _DownloadRing(this.progress,
      {required this.track, required this.arc, required this.arrow});

  @override
  void paint(Canvas canvas, Size size) {
    // Drawn on the set's 16px grid at its 1.5px weight.
    canvas.scale(size.width / 16);
    final pen = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round
      ..color = track;
    final ring = Rect.fromCircle(center: const Offset(8, 8), radius: 6.75);
    canvas.drawOval(ring, pen);
    canvas.drawArc(
        ring, -math.pi / 2, math.pi * 2 * progress, false, pen..color = arc);
    canvas.drawPath(
      Path()
        ..moveTo(8, 4.75)
        ..lineTo(8, 10.75)
        ..moveTo(5.75, 8.5)
        ..lineTo(8, 10.75)
        ..lineTo(10.25, 8.5),
      pen..color = arrow,
    );
  }

  @override
  bool shouldRepaint(_DownloadRing old) =>
      old.progress != progress ||
      old.track != track ||
      old.arc != arc ||
      old.arrow != arrow;
}
