// The easing editor: a unit box you shape an ease in, and a button that stamps
// it onto the selected keyframes (docs/07 §5.3 and §5.4, K-348 and K-349).
//
// In plain terms: the graph editor shapes one span at a time, in the units that
// span happens to use. This is the same shape drawn once, in the abstract — the
// travel runs left to right across the box, from nothing done to all done, and
// the two handles bend it. Because the box is unitless the shape is reusable:
// Apply puts it on every span the selection covers, whatever those spans move
// by (`applyEasingToSelection`, the per-span conversion it leans on).
//
// **One editor, shown two ways** (K-349). By default it is the *Easing panel*
// (`easing_panel_frb.dart`), which stays on screen while the selection changes
// underneath it — that is the whole reason to prefer a panel. Settings ▸
// Interface ▸ Editing turns it back into a popup that opens from the graph's
// own bottom bar and closes when it is done. [EasingEditor] is the body both
// show, and knows about neither: the only differences are whether there is a
// Close button and whether Apply has anywhere to send a shape.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'easing_curve.dart';

/// A preset's shown name. Kept beside the popup rather than in
/// `easing_curve.dart` so the maths stays free of the locale.
String easingPresetName(String id) => switch (id) {
      'easy' => l10n.easeEasy,
      'slowStart' => l10n.easingSlowStart,
      'slowFinish' => l10n.easingSlowFinish,
      'heavy' => l10n.easingHeavy,
      'snap' => l10n.easingSnap,
      'overshoot' => l10n.easingOvershoot,
      _ => l10n.easingAnticipate,
    };

/// The unit box, in logical pixels — the square the travel runs across.
const double _boxSide = 170;

/// Side room: enough for a knob at x = 0 or x = 1 to be drawn whole.
const double _marginX = 20;

/// Room above and below, sized from [easingHandleReach] rather than guessed, so
/// a handle at the very limit still draws inside the surface. Getting this
/// wrong is not cosmetic: a knob past the edge is one the pointer cannot reach
/// to drag back.
const double _marginY = _boxSide * easingHandleReach;

/// How near the pointer must come to a handle to take hold of it.
const double _grabRadius = 18;

/// The content size: the box and its margins, and what every row inside the
/// popup is pinned to.
const double _popupWidth = _boxSide + _marginX * 2;
const double _popupHeight = _boxSide + _marginY * 2;

/// Open the easing editor at [position] as a popup, and call [onApply] with the
/// shape each time Apply is pressed — the *inline* mode of Settings ▸ Interface
/// ▸ Editing (K-349).
///
/// The popup stays up across an Apply — shaping an ease is a "try it, nudge it,
/// try it again" job, and a box that vanished on first use would make the
/// second attempt start from nothing. It closes on Close, or on a click outside
/// it, like every other popup here. That last part is why the panel is the
/// default: changing the keyframe selection *is* a click outside, so in popup
/// mode one shape can only ever be tried on one selection.
Future<void> showEasingPopup({
  required BuildContext context,
  required Offset position,
  required ValueChanged<EasingCurve> onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => Builder(builder: (context) {
        // The floating surface belongs to the popup, not to the editor: a panel
        // draws its own chrome and a second raised card inside one is a card
        // too many.
        final t = ThemeScope.of(context).theme;
        return Container(
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.tokens.floatRadius),
            border: Border.all(color: t.hairlineStrong),
          ),
          child: EasingEditor(
            onApply: onApply,
            onClose: () => close(null),
          ),
        );
      }),
    );

/// The editor body: the box, the preset row, the four numbers, and the buttons.
///
/// [onClose] null means there is no Close button — the panel is not something
/// you dismiss. [onApply] null means Apply is there but greyed, with [whyNot]
/// saying what would make it live again; the panel is persistent, so a button
/// that silently did nothing would read as a fault rather than a lock.
class EasingEditor extends StatefulWidget {
  final ValueChanged<EasingCurve>? onApply;
  final VoidCallback? onClose;

  /// Shown under the buttons while [onApply] is null.
  final String? whyNot;

  const EasingEditor({
    super.key,
    required this.onApply,
    this.onClose,
    this.whyNot,
  });

  @override
  State<EasingEditor> createState() => _EasingEditorState();
}

class _EasingEditorState extends State<EasingEditor> {
  /// It opens on the gentlest preset. The shape survives a selection change in
  /// the panel — the State is not rebuilt for one — which is the thing the
  /// popup could not do.
  EasingCurve _curve = easingPresets.first.curve;

  /// Which handle the pointer has hold of: 1, 2, or null between drags.
  int? _dragging;

  /// The box's drawing rect inside this widget's own coordinates.
  Rect get _box => Rect.fromLTWH(_marginX, _marginY, _boxSide, _boxSide);

  /// A control point in widget coordinates. y is flipped: the curve rises as
  /// the value completes, and screen y grows downward.
  Offset _pointOf(int handle) {
    final b = _box;
    final x = handle == 1 ? _curve.x1 : _curve.x2;
    final y = handle == 1 ? _curve.y1 : _curve.y2;
    return Offset(b.left + x * b.width, b.bottom - y * b.height);
  }

  /// A widget-coordinate position back into curve space, undoing the flip.
  ({double x, double y}) _curveSpace(Offset local) {
    final b = _box;
    return (
      x: (local.dx - b.left) / b.width,
      y: (b.bottom - local.dy) / b.height,
    );
  }

  void _grab(Offset local) {
    final d1 = (local - _pointOf(1)).distance;
    final d2 = (local - _pointOf(2)).distance;
    if (d1 > _grabRadius && d2 > _grabRadius) return;
    setState(() => _dragging = d1 <= d2 ? 1 : 2);
    _drag(local);
  }

  void _drag(Offset local) {
    final handle = _dragging;
    if (handle == null) return;
    final p = _curveSpace(local);
    // x is clamped by EasingCurve itself (the span has to stay x-monotone);
    // y is left exactly where the pointer put it, so overshoot is drawable.
    setState(
        () => _curve = _curve.withHandle(first: handle == 1, x: p.x, y: p.y));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final apply = widget.onApply;
    return Padding(
      padding: const EdgeInsets.all(10),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Listener(
            onPointerDown: (e) => _grab(e.localPosition),
            onPointerMove: (e) => _drag(e.localPosition),
            onPointerUp: (_) => setState(() => _dragging = null),
            onPointerCancel: (_) => setState(() => _dragging = null),
            child: CustomPaint(
              size: const Size(_popupWidth, _popupHeight),
              painter: _EasingPainter(curve: _curve, box: _box, theme: t),
            ),
          ),
          const SizedBox(height: 8),
          _presetRow(t),
          const SizedBox(height: 8),
          // The four numbers, so a shape can be read off and typed back in
          // elsewhere. Two decimals is the precision the box can be dragged to.
          Text(
            '${_curve.x1.toStringAsFixed(2)}, ${_curve.y1.toStringAsFixed(2)}, '
            '${_curve.x2.toStringAsFixed(2)}, ${_curve.y2.toStringAsFixed(2)}',
            style: t.mono.copyWith(color: t.textMuted),
          ),
          const SizedBox(height: 6),
          // Width pinned to the box: a Row with a Spacer takes every pixel it
          // is offered, and inside an overlay that is the whole window.
          SizedBox(
            width: _popupWidth,
            child: Row(
              children: [
                const Spacer(),
                if (widget.onClose != null) ...[
                  HouseButton(
                    small: true,
                    onPressed: widget.onClose,
                    child: Text(l10n.close, style: t.small),
                  ),
                  const SizedBox(width: 6),
                ],
                HouseButton(
                  key: const ValueKey('easing-apply'),
                  small: true,
                  primary: true,
                  // A null callback is what greys a HouseButton, so the lock
                  // and the look are the same fact rather than two.
                  onPressed: apply == null ? null : () => apply(_curve),
                  child: Text(l10n.apply,
                      style: t.small.copyWith(
                          color:
                              apply == null ? t.textDisabled : t.textPrimary)),
                ),
              ],
            ),
          ),
          if (apply == null && widget.whyNot != null) ...[
            const SizedBox(height: 6),
            SizedBox(
              width: _popupWidth,
              child: Text(widget.whyNot!,
                  style: t.caption.copyWith(color: t.textMuted)),
            ),
          ],
        ],
      ),
    );
  }

  /// The shipped shapes, each a button that loads it into the box. Loading
  /// rather than applying: a preset is a starting point to nudge, and Apply
  /// stays the one thing that touches the document.
  Widget _presetRow(LumitTheme t) => SizedBox(
        width: _popupWidth,
        child: Wrap(
          spacing: 4,
          runSpacing: 4,
          children: [
            for (final preset in easingPresets)
              HouseButton(
                small: true,
                frameless: true,
                padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 2),
                onPressed: () => setState(() => _curve = preset.curve),
                child: Text(
                  easingPresetName(preset.id),
                  style: t.caption.copyWith(
                    color: _curve == preset.curve ? t.accent : t.textMuted,
                  ),
                ),
              ),
          ],
        ),
      );
}

class _EasingPainter extends CustomPainter {
  final EasingCurve curve;
  final Rect box;
  final LumitTheme theme;

  const _EasingPainter({
    required this.curve,
    required this.box,
    required this.theme,
  });

  Offset _at(double x, double y) =>
      Offset(box.left + x * box.width, box.bottom - y * box.height);

  @override
  void paint(Canvas canvas, Size size) {
    final hairline = Paint()
      ..color = theme.hairline
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke;

    // The box, and the chord across it: the ease measured against no ease at
    // all, which is the comparison the shape is read against.
    canvas.drawRect(box, hairline);
    canvas.drawLine(
      _at(0, 0),
      _at(1, 1),
      Paint()
        ..color = theme.hairline
        ..strokeWidth = 1,
    );

    final p1 = _at(curve.x1, curve.y1);
    final p2 = _at(curve.x2, curve.y2);

    // Handle stems first, so the curve and the knobs sit over them.
    final stem = Paint()
      ..color = theme.textDisabled
      ..strokeWidth = 1;
    canvas.drawLine(_at(0, 0), p1, stem);
    canvas.drawLine(_at(1, 1), p2, stem);

    // The shape itself. Walked in u rather than solved per x: this is the
    // drawn curve, not a sampled one (see EasingCurve.yAt).
    final path = Path()..moveTo(_at(0, 0).dx, _at(0, 0).dy);
    const steps = 64;
    for (var i = 1; i <= steps; i++) {
      final u = i / steps;
      final p = _at(curve.xAt(u), curve.yAt(u));
      path.lineTo(p.dx, p.dy);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = theme.curve.first
        ..strokeWidth = 2
        ..style = PaintingStyle.stroke,
    );

    final knob = Paint()..color = theme.accent;
    canvas.drawCircle(p1, 4.5, knob);
    canvas.drawCircle(p2, 4.5, knob);
  }

  @override
  bool shouldRepaint(_EasingPainter old) =>
      old.curve != curve || old.box != box || old.theme != theme;
}
