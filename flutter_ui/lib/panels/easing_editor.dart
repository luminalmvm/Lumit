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
//
// **Shapes of your own.** Save… keeps the drawn curve under a name, and it then
// stands in the preset row beside the seven that ship, applying by exactly the
// same road. Those saved ones belong to the person rather than to the project,
// so they live beside the settings file — see `state/custom_easings.dart`.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/custom_easings.dart';
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
      'sineIn' => l10n.easingSineIn,
      'sineOut' => l10n.easingSineOut,
      'sineInOut' => l10n.easingSineInOut,
      'quadIn' => l10n.easingQuadIn,
      'quadOut' => l10n.easingQuadOut,
      'quadInOut' => l10n.easingQuadInOut,
      'cubicIn' => l10n.easingCubicIn,
      'cubicOut' => l10n.easingCubicOut,
      'cubicInOut' => l10n.easingCubicInOut,
      'quartIn' => l10n.easingQuartIn,
      'quartOut' => l10n.easingQuartOut,
      'quartInOut' => l10n.easingQuartInOut,
      'expoIn' => l10n.easingExpoIn,
      'expoOut' => l10n.easingExpoOut,
      'expoInOut' => l10n.easingExpoInOut,
      'backIn' => l10n.easingBackIn,
      'backOut' => l10n.easingBackOut,
      _ => l10n.easingBackInOut,
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

/// The preset grid's tiles: three to a row of the content width.
const double _tileGap = 3;
const double _tileWidth = (_popupWidth - _tileGap * 2) / 3;

/// A tile's curve thumbnail height. Its drawn unit box leaves a margin above
/// and below for the Back family's overshoot; the last pixel or two of a full
/// overshoot clips, which is what the tile's ClipRect is for.
const double _thumbHeight = 32;

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
          // The preset grid makes the editor taller than a popup should be:
          // past this it scrolls, where the docked panel scrolls for itself.
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 460),
            child: SingleChildScrollView(
              child: EasingEditor(
                onApply: onApply,
                onClose: () => close(null),
              ),
            ),
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

  /// What the name field is being used for: null for "it is not there", the
  /// empty string for saving the drawn shape, and a saved shape's name for
  /// renaming that one. One field rather than two booleans because the field
  /// itself is one thing, and it is never doing both jobs at once.
  String? _naming;

  final TextEditingController _nameField = TextEditingController();

  @override
  void dispose() {
    _nameField.dispose();
    super.dispose();
  }

  /// Put the name field up, filled with whatever the thing is called now.
  void _startNaming(String forName) => setState(() {
        _naming = forName;
        _nameField.text = forName;
      });

  /// Enter, or a click away: keep what was typed. A blank name keeps nothing —
  /// [CustomEasings] refuses it — and the field simply closes.
  void _commitName() {
    final job = _naming;
    if (job == null) return;
    setState(() {
      if (job.isEmpty) {
        CustomEasings.add(_nameField.text, _curve);
      } else {
        CustomEasings.rename(job, _nameField.text);
      }
      _naming = null;
    });
  }

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
          // The empty drag handlers are not dead code: they put the box in the
          // gesture arena, where the inner member beats the panel's scroll
          // view — the preset grid made the panel tall enough to scroll, and a
          // handle drag must shape the curve, not scroll it away. The Listener
          // does the actual work, because it hears the very first pointer-down
          // rather than waiting out the drag slop.
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: (_) {},
            onHorizontalDragUpdate: (_) {},
            child: Listener(
              key: const ValueKey('easing-box'),
              onPointerDown: (e) => _grab(e.localPosition),
              onPointerMove: (e) => _drag(e.localPosition),
              onPointerUp: (_) => setState(() => _dragging = null),
              onPointerCancel: (_) => setState(() => _dragging = null),
              child: CustomPaint(
                size: const Size(_popupWidth, _popupHeight),
                painter: _EasingPainter(curve: _curve, box: _box, theme: t),
              ),
            ),
          ),
          const SizedBox(height: 8),
          _presetGrid(t),
          if (_naming != null) ...[
            const SizedBox(height: 6),
            HouseTextField(
              key: const ValueKey('easing-name'),
              controller: _nameField,
              width: _popupWidth,
              autofocus: true,
              hint: l10n.easingNameYours,
              onSubmitted: (_) => _commitName(),
              onTapOutside: _commitName,
              onCancelled: () => setState(() => _naming = null),
            ),
          ],
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
                // Keeping the drawn shape is not applying it: Apply stays the
                // one button that touches the document.
                HouseButton(
                  key: const ValueKey('easing-save'),
                  small: true,
                  onPressed: () => _startNaming(''),
                  child: Text(l10n.easingSaveEllipsis, style: t.small),
                ),
                const SizedBox(width: 6),
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

  /// The preset library (K-726): the shipped shapes and the user's own, each a
  /// tile drawing its curve with its name under it — the grid the panel is
  /// mostly made of, the way Flow's library is.
  ///
  /// **A click is an apply.** The tile loads its shape into the box *and*
  /// presses the same road the Apply button presses, so easing a selection is
  /// one click, not a pick and a confirm. While there is nowhere to send a
  /// shape (Apply is grey) a click only loads, and the box is a place to look
  /// at the curve.
  ///
  /// The saved ones sit in the same grid, after the shipped shapes, and behave
  /// the same in every way but one: they can be renamed and thrown away, from a
  /// right-click on the tile itself. A shipped preset has no such menu — there
  /// is nothing there the user put in.
  Widget _presetGrid(LumitTheme t) => SizedBox(
        width: _popupWidth,
        child: Wrap(
          spacing: _tileGap,
          runSpacing: _tileGap,
          children: [
            for (final preset in easingPresets)
              _tile(t, easingPresetName(preset.id), preset.curve),
            for (final saved in CustomEasings.all)
              HouseContextMenu(
                itemBuilder: (close) => [
                  MenuRow(
                    onPressed: () {
                      close();
                      _startNaming(saved.name);
                    },
                    child: Text(l10n.rename, style: t.small),
                  ),
                  MenuRow(
                    onPressed: () {
                      close();
                      setState(() => CustomEasings.delete(saved.name));
                    },
                    child: Text(l10n.delete, style: t.small),
                  ),
                ],
                child: _tile(t, saved.name, saved.curve),
              ),
          ],
        ),
      );

  Widget _tile(LumitTheme t, String name, EasingCurve curve) {
    final current = _curve == curve;
    return HouseButton(
      small: true,
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
      onPressed: () {
        setState(() => _curve = curve);
        widget.onApply?.call(curve);
      },
      child: SizedBox(
        width: _tileWidth - 8,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            // The thumbnail clips: an overshooting curve may bend past the
            // drawn margin, and the caption under it is not the place for it.
            ClipRect(
              child: CustomPaint(
                size: const Size(_tileWidth - 8, _thumbHeight),
                painter: _TilePainter(
                  curve: curve,
                  theme: t,
                  current: current,
                ),
              ),
            ),
            const SizedBox(height: 2),
            Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: t.caption.copyWith(
                color: current ? t.accent : t.textMuted,
              ),
            ),
          ],
        ),
      ),
    );
  }
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

/// A preset tile's thumbnail: the chord as a hairline and the curve over it,
/// walked by the same [EasingCurve.xAt]/[EasingCurve.yAt] the editor's box
/// walks — the tile draws the very shape Apply would stamp, not a picture of
/// one.
class _TilePainter extends CustomPainter {
  final EasingCurve curve;
  final LumitTheme theme;
  final bool current;

  const _TilePainter({
    required this.curve,
    required this.theme,
    required this.current,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final box = Rect.fromLTWH(2, 7, size.width - 4, size.height - 14);
    Offset at(double x, double y) =>
        Offset(box.left + x * box.width, box.bottom - y * box.height);

    canvas.drawLine(
      at(0, 0),
      at(1, 1),
      Paint()
        ..color = theme.hairline
        ..strokeWidth = 1,
    );

    final path = Path()..moveTo(at(0, 0).dx, at(0, 0).dy);
    // Half the editor's walk: the tile is a quarter the size, and 32 straight
    // pieces are already below a pixel each here.
    const steps = 32;
    for (var i = 1; i <= steps; i++) {
      final u = i / steps;
      final p = at(curve.xAt(u), curve.yAt(u));
      path.lineTo(p.dx, p.dy);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = current ? theme.accent : theme.curve.first
        ..strokeWidth = 1.5
        ..style = PaintingStyle.stroke,
    );
  }

  @override
  bool shouldRepaint(_TilePainter old) =>
      old.curve != curve || old.theme != theme || old.current != current;
}
