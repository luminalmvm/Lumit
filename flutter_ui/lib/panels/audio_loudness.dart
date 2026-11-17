// The master's loudness as quiet numbers, under the Audio panel's levels and
// beside the Mixer's Master meter: short-term, integrated and the true peak,
// and on the Master the momentary loudness and the range too. A click on the
// integrated number starts it afresh.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

import '../l10n/strings.dart';
import '../state/ui_state.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';

/// How the numbers stand: side by side, stacked with each mark over its number
/// for a strip, or the integrated number alone where a strip has no room.
enum LoudnessLayout { row, column, integrated }

class LoudnessReadout extends StatelessWidget {
  final AudioMeterFeed feed;

  final LoudnessLayout layout;

  /// The comp's loudness target in LUFS, shown beside the integrated mark and
  /// read against it, or null when the comp has none.
  final double? target;

  /// Whether the target stands after the integrated reading, where there is
  /// room for it.
  final bool targetBeside;

  const LoudnessReadout(
      {super.key,
      required this.feed,
      this.layout = LoudnessLayout.row,
      this.target,
      this.targetBeside = true});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final mark = t.mono.copyWith(fontSize: 9, color: t.textMuted);
    final aim = target;
    return ValueListenableBuilder<LoudnessLevels>(
      valueListenable: feed.loudness,
      builder: (context, l, _) {
        Widget cell(String key, String letters, String name, double value,
            {VoidCallback? onTap, Color? ink, String? after}) {
          return LumitTooltip(
            message: name,
            child: HoverMark(
              cursor:
                  onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
              builder: (context, lit) {
                final number = Text(
                  value.isFinite ? value.toStringAsFixed(1) : l10n.volumeNegInf,
                  key: ValueKey<String>(key),
                  maxLines: 1,
                  softWrap: false,
                  // Nothing measured recedes, like the label beside it.
                  style: mark.copyWith(
                    color: Color.lerp(
                        value.isFinite ? ink ?? t.textSecondary : t.textMuted,
                        t.textPrimary,
                        onTap == null ? 0 : lit),
                  ),
                );
                final words =
                    Text(letters, maxLines: 1, softWrap: false, style: mark);
                // The target, quiet, after the reading it is read against.
                final reading = after == null
                    ? number
                    : Row(mainAxisSize: MainAxisSize.min, children: [
                        number,
                        const SizedBox(width: 4),
                        Text(after,
                            maxLines: 1, softWrap: false, style: mark),
                      ]);
                final body = layout == LoudnessLayout.column
                    ? Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [words, reading],
                      )
                    : Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [words, const SizedBox(width: 4), reading],
                      );
                return GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: onTap,
                  child: body,
                );
              },
            ),
          );
        }

        // Within a unit of the target is on it and louder is over it;
        // quieter reads as any other number does.
        final i = l.integrated;
        final ink = aim == null || !i.isFinite
            ? null
            : (i - aim).abs() <= 1
                ? t.success
                : i > aim
                    ? t.warning
                    : null;
        final integrated = cell('loudness-integrated',
            l10n.loudnessIntegratedMark, l10n.loudnessIntegrated, i,
            onTap: resetLoudness,
            ink: ink,
            after: aim == null || !targetBeside
                ? null
                : aim.toStringAsFixed(aim.truncateToDouble() == aim ? 0 : 1));
        if (layout == LoudnessLayout.integrated) return integrated;
        final column = layout == LoudnessLayout.column;
        final cells = [
          if (column)
            cell('loudness-momentary', l10n.loudnessMomentaryMark,
                l10n.loudnessMomentary, l.momentary),
          cell('loudness-short', l10n.loudnessShortTermMark,
              l10n.loudnessShortTerm, l.shortTerm),
          integrated,
          cell('loudness-peak', l10n.loudnessTruePeakMark,
              l10n.loudnessTruePeak, l.truePeak),
          if (column)
            cell('loudness-range', l10n.loudnessRangeMark,
                l10n.loudnessRange, l.range),
        ];
        return column
            ? Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var i = 0; i < cells.length; i++) ...[
                    if (i > 0) const SizedBox(height: 6),
                    cells[i],
                  ],
                ],
              )
            : Row(
                children: [
                  for (var i = 0; i < cells.length; i++) ...[
                    if (i > 0) const SizedBox(width: 14),
                    cells[i],
                  ],
                ],
              );
      },
    );
  }
}

/// The master's short-term loudness over the last half minute of playing,
/// filling from the left and then moving along, on a quiet LUFS scale with
/// the comp's loudness target as a line across it.
/// It starts afresh with Reset loudness and when another comp comes up.
class LoudnessHistoryGraph extends StatefulWidget {
  final AudioMeterFeed feed;
  final LumitUiState ui;

  /// How far in from the left the trace starts, so it lines up with the bars
  /// above it. At the right it leaves the room their numbers take.
  final double left;

  const LoudnessHistoryGraph({
    super.key,
    required this.feed,
    required this.ui,
    this.left = 0,
  });

  @override
  State<LoudnessHistoryGraph> createState() => _LoudnessHistoryGraphState();
}

class _LoudnessHistoryGraphState extends State<LoudnessHistoryGraph> {
  /// The comp drawn for and its target, read when the document moves and
  /// never while drawing.
  CompositionReference? _comp;
  double? _target;

  @override
  void initState() {
    super.initState();
    widget.ui.model.addListener(_read);
    _comp = widget.ui.selectedComp;
    _target = _targetOf(_comp);
  }

  @override
  void didUpdateWidget(LoudnessHistoryGraph old) {
    super.didUpdateWidget(old);
    // Another comp up: its loudness is not this one's.
    if (widget.ui.selectedComp?.internalid != _comp?.internalid) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _read();
      });
    }
  }

  @override
  void dispose() {
    widget.ui.model.removeListener(_read);
    super.dispose();
  }

  static double? _targetOf(CompositionReference? comp) {
    try {
      return comp?.loudnessTarget()?.lufs;
    } catch (_) {
      return null;
    }
  }

  void _read() {
    final comp = widget.ui.selectedComp;
    if (comp?.internalid != _comp?.internalid) widget.feed.history.clear();
    _comp = comp;
    final target = _targetOf(comp);
    if (target != _target) setState(() => _target = target);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: l10n.loudnessShortTerm,
      child: RepaintBoundary(
        key: const ValueKey('loudness-history'),
        child: SizedBox(
          height: 40,
          width: double.infinity,
          child: CustomPaint(
            painter: _HistoryPainter(
              history: widget.feed.history,
              target: _target,
              left: widget.left,
              ground: t.surface2,
              grid: t.hairline,
              trace: t.accent,
              aim: t.textMuted,
              text: t.mono.copyWith(fontSize: 9, color: t.textMuted),
              scale: t.caption.copyWith(color: t.textMuted),
              aimWords: t.caption.copyWith(color: t.textSecondary),
            ),
          ),
        ),
      ),
    );
  }
}

class _HistoryPainter extends CustomPainter {
  /// The loudness the graph spans, top and bottom, in LUFS.
  static const double _top = -6, _bottom = -42;

  final LoudnessHistory history;
  final double? target;
  final double left;
  final Color ground, grid, trace, aim;

  /// The reading's face, which the right-hand room is measured in, and the
  /// faces of the scale and the target beside the graph.
  final TextStyle text, scale, aimWords;

  _HistoryPainter({
    required this.history,
    required this.target,
    required this.left,
    required this.ground,
    required this.grid,
    required this.trace,
    required this.aim,
    required this.text,
    required this.scale,
    required this.aimWords,
  }) : super(repaint: history);

  /// The room a reading takes at the right, as the level bars leave it.
  late final double _right = () {
    final widest = TextPainter(
      text: TextSpan(text: '-00.0', style: text),
      textDirection: TextDirection.ltr,
    )..layout();
    final room = widest.width + 6;
    widest.dispose();
    return room;
  }();

  @override
  void paint(Canvas canvas, Size size) {
    final plot = Rect.fromLTRB(left, 0, size.width - _right, size.height);
    double yOf(double lufs) =>
        plot.top +
        plot.height * ((_top - lufs) / (_top - _bottom)).clamp(0.0, 1.0);
    canvas.drawRect(plot, Paint()..color = ground);
    void word(String words, double y, double x, TextStyle style) {
      final painter = TextPainter(
        text: TextSpan(text: words, style: style),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(canvas, Offset(x, y - painter.height / 2));
      painter.dispose();
    }

    final aimAt = target;
    for (final lufs in const [-12.0, -24.0, -36.0]) {
      final y = yOf(lufs);
      canvas.drawRect(
          Rect.fromLTWH(plot.left, y - 0.5, plot.width, 1),
          Paint()..color = grid);
      // A mark the target's own number would sit on gives way to it.
      if (aimAt == null || (yOf(aimAt) - y).abs() > 9) {
        word('${lufs.round()}', y, plot.right + 4, scale);
      }
    }
    if (aimAt != null) {
      final y = yOf(aimAt);
      canvas.drawRect(Rect.fromLTWH(plot.left, y - 0.5, plot.width, 1),
          Paint()..color = aim);
      word(aimAt.toStringAsFixed(aimAt.truncateToDouble() == aimAt ? 0 : 1),
          y, plot.right + 4, aimWords);
    }
    // The first reading at the left edge, a step per reading, broken where
    // nothing was measured. A full window moves along.
    final step = plot.width / (LoudnessHistory.length - 1);
    final path = Path();
    var open = false;
    for (var i = 0; i < history.count; i++) {
      final lufs = history[i];
      if (!lufs.isFinite || lufs < _bottom - 20) {
        open = false;
        continue;
      }
      final at = Offset(plot.left + i * step, yOf(lufs));
      if (open) {
        path.lineTo(at.dx, at.dy);
      } else {
        path.moveTo(at.dx, at.dy);
        open = true;
      }
    }
    canvas.save();
    canvas.clipRect(plot);
    canvas.drawPath(
        path,
        Paint()
          ..color = trace
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5);
    canvas.restore();
  }

  @override
  bool shouldRepaint(_HistoryPainter old) =>
      old.history != history ||
      old.target != target ||
      old.trace != trace ||
      old.scale != scale ||
      old.left != left;
}
