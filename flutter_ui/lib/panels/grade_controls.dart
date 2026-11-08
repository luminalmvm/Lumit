// The Grade panel's rows: a section heading with its reset, a slider row, and
// the right-click holder they share. The panel decides what each one writes.

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/widgets.dart';

import '../icons/icons.dart' show iconSize;
import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show DoubleTap;

/// The room every row keeps from the panel's edge.
const double gradeInset = 10;

/// A right-click holder that floats [rows] at the pointer. The rows are built
/// on the click, so they read the document as it stands then.
class GradeMenu extends StatelessWidget {
  final List<Widget> Function(VoidCallback close) rows;
  final Widget child;

  const GradeMenu({super.key, required this.rows, required this.child});

  @override
  Widget build(BuildContext context) => GestureDetector(
        behavior: HitTestBehavior.translucent,
        onSecondaryTapDown: (d) => show(context, d.globalPosition, rows),
        child: child,
      );

  static void show(BuildContext context, Offset at,
      List<Widget> Function(VoidCallback close) rows) {
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: rows(() => close(null)),
          ),
        ),
      ),
    );
  }
}

/// A section's name, and its reset where the section has left its defaults.
/// The reset is the only control on the line: quiet until the pointer is
/// over it, gone while there is nothing to put back.
class GradeHeading extends StatelessWidget {
  final String label;
  final bool changed;
  final VoidCallback? onReset;

  const GradeHeading(
      {super.key,
      required this.label,
      required this.changed,
      required this.onReset});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(gradeInset, 8, 4, 2),
      child: SizedBox(
        height: 18,
        child: Row(
          children: [
            Expanded(
              child: Text(label,
                  style: t.kicker, maxLines: 1, overflow: TextOverflow.clip),
            ),
            if (changed && onReset != null)
              GradeIconButton(
                key: ValueKey<String>('grade-reset-$label'),
                glyph: LumitIcons.reset,
                tip: l10n.reset,
                onPressed: onReset!,
              ),
          ],
        ),
      ),
    );
  }
}

/// A small frameless icon button: muted at rest, bright under the pointer.
class GradeIconButton extends StatelessWidget {
  final String glyph;
  final String tip;
  final VoidCallback? onPressed;

  /// Lit, for a button that is a held state rather than a press.
  final bool active;

  const GradeIconButton(
      {super.key,
      required this.glyph,
      required this.tip,
      required this.onPressed,
      this.active = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: tip,
      child: HouseButton(
        frameless: true,
        small: true,
        active: active,
        padding: const EdgeInsets.all(2),
        onPressed: onPressed,
        child: HoverMark(
          builder: (context, lit) => LumitIcon(
            glyph,
            size: iconSize,
            colour: onPressed == null
                ? t.textDisabled
                : active
                    ? null
                    : Color.lerp(t.textSecondary, t.textPrimary, lit),
          ),
        ),
      ),
    );
  }
}

/// One primary: its name, a track, and the number.
///
/// The track is the whole row's width between the name and the number, and
/// the pointer sets the value where it lands: a slider here is a thing to
/// grab, not a thing to find. The fill runs from the neutral value, so a row
/// that has moved shows how far and which way, and a row at its default
/// shows nothing but its line. Below a width that can hold a track, the name
/// and the number stay and the track goes.
class GradeSlider extends StatefulWidget {
  final String label;
  final double value;
  final double min, max;

  /// What the fill measures from, and what a double-click puts back.
  final double neutral;
  final int decimals;
  final bool signed;
  final bool enabled;
  final bool keyed;

  /// The start of a gesture, before anything moves.
  final VoidCallback onStart;
  final ValueChanged<double> onLive;
  final ValueChanged<double> onCommit;

  /// A gesture that began and wrote nothing.
  final VoidCallback onCancel;

  /// The rows of the right-click menu, or none for a row with no menu.
  final List<Widget> Function(VoidCallback close)? menu;

  const GradeSlider({
    super.key,
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.neutral,
    required this.onStart,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
    this.decimals = 1,
    this.signed = false,
    this.enabled = true,
    this.keyed = false,
    this.menu,
  });

  @override
  State<GradeSlider> createState() => _GradeSliderState();
}

class _GradeSliderState extends State<GradeSlider> {
  /// The value in hand during a drag on the track.
  double? _live;
  bool _over = false;

  /// Counted by hand rather than through `onDoubleTap`, which would hold
  /// every single tap back while it waited for a second.
  final DoubleTap _taps = DoubleTap();

  double _at(double x, double width) =>
      widget.min + (x / width).clamp(0.0, 1.0) * (widget.max - widget.min);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final value = _live ?? widget.value;
    final changed = (widget.value - widget.neutral).abs() > 1e-6;
    final labelColour = widget.keyed
        ? t.animated
        : widget.enabled
            ? (changed ? t.textSecondary : t.textMuted)
            : t.textDisabled;
    final row = LayoutBuilder(builder: (context, box) {
      final wide = box.maxWidth >= 200;
      return Row(
        children: [
          SizedBox(
            width: 74,
            child: Text(widget.label,
                style: t.small.copyWith(color: labelColour),
                maxLines: 1,
                overflow: TextOverflow.ellipsis),
          ),
          if (wide)
            Expanded(
              child: LayoutBuilder(builder: (context, track) {
                final width = track.maxWidth;
                return MouseRegion(
                  cursor: widget.enabled
                      ? SystemMouseCursors.resizeLeftRight
                      : MouseCursor.defer,
                  onEnter: (_) => setState(() => _over = true),
                  onExit: (_) => setState(() => _over = false),
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    dragStartBehavior: DragStartBehavior.down,
                    onTapUp: !widget.enabled
                        ? null
                        : (d) {
                            final again =
                                _taps.tap(at: d.localPosition, slop: 6);
                            widget.onStart();
                            widget.onCommit(again
                                ? widget.neutral
                                : _at(d.localPosition.dx, width));
                          },
                    onHorizontalDragStart: !widget.enabled
                        ? null
                        : (d) {
                            widget.onStart();
                            setState(
                                () => _live = _at(d.localPosition.dx, width));
                            widget.onLive(_live!);
                          },
                    onHorizontalDragUpdate: !widget.enabled
                        ? null
                        : (d) {
                            setState(
                                () => _live = _at(d.localPosition.dx, width));
                            widget.onLive(_live!);
                          },
                    onHorizontalDragEnd: !widget.enabled
                        ? null
                        : (_) {
                            final v = _live ?? widget.value;
                            setState(() => _live = null);
                            widget.onCommit(v);
                          },
                    onHorizontalDragCancel: !widget.enabled
                        ? null
                        : () {
                            setState(() => _live = null);
                            widget.onCancel();
                          },
                    child: SizedBox(
                      height: 22,
                      child: CustomPaint(
                        painter: _TrackPainter(
                          at: ((value - widget.min) / (widget.max - widget.min))
                              .clamp(0.0, 1.0),
                          from: ((widget.neutral - widget.min) /
                                  (widget.max - widget.min))
                              .clamp(0.0, 1.0),
                          track: widget.enabled ? t.hairlineStrong : t.hairline,
                          fill: !widget.enabled
                              ? t.hairline
                              : widget.keyed
                                  ? t.animated
                                  : t.accent,
                          knob: !widget.enabled
                              ? t.textDisabled
                              : _live != null
                                  ? t.accent
                                  : widget.keyed
                                      ? t.animated
                                      : (_over
                                          ? t.textPrimary
                                          : t.textSecondary),
                        ),
                      ),
                    ),
                  ),
                );
              }),
            )
          else
            const Spacer(),
          const SizedBox(width: 8),
          SizedBox(
            width: 60,
            child: IgnorePointer(
              ignoring: !widget.enabled,
              child: Opacity(
                opacity: widget.enabled ? 1 : 0.5,
                child: DragValueField(
                  key: ValueKey<String>('grade-${widget.label}'),
                  value: value,
                  min: widget.min,
                  max: widget.max,
                  speed: (widget.max - widget.min) / 200,
                  decimals: widget.decimals,
                  signed: widget.signed,
                  keyed: widget.keyed,
                  resetTo: widget.neutral,
                  onChangeStart: widget.onStart,
                  onChangeLive: (v) => widget.onLive(v.toDouble()),
                  onChangeEnd: (v) => widget.onCommit(v.toDouble()),
                  onDragCancel: widget.onCancel,
                  onChanged: (v) {
                    widget.onStart();
                    widget.onCommit(v.toDouble());
                  },
                ),
              ),
            ),
          ),
        ],
      );
    });
    final body = Padding(
      padding: const EdgeInsets.symmetric(horizontal: gradeInset, vertical: 1),
      child: row,
    );
    final menu = widget.menu;
    return menu == null ? body : GradeMenu(rows: menu, child: body);
  }
}

/// A track with the fill from the neutral point to the knob.
class _TrackPainter extends CustomPainter {
  final double at, from;
  final Color track, fill, knob;

  const _TrackPainter({
    required this.at,
    required this.from,
    required this.track,
    required this.fill,
    required this.knob,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final y = size.height / 2;
    final inset = 4.0;
    final w = size.width - inset * 2;
    canvas.drawLine(
      Offset(inset, y),
      Offset(inset + w, y),
      Paint()
        ..color = track
        ..strokeWidth = 1,
    );
    final x0 = inset + from * w;
    final x1 = inset + at * w;
    if ((x1 - x0).abs() > 0.5) {
      canvas.drawLine(
        Offset(x0, y),
        Offset(x1, y),
        Paint()
          ..color = fill
          ..strokeWidth = 2,
      );
    }
    canvas.drawCircle(Offset(x1, y), 3.5, Paint()..color = knob);
  }

  @override
  bool shouldRepaint(_TrackPainter old) =>
      old.at != at ||
      old.from != from ||
      old.fill != fill ||
      old.knob != knob ||
      old.track != track;
}
