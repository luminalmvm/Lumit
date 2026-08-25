// Drag-a-box selection, shared by the Timeline's lanes and the graph editor.
//
// In plain terms: put this as a `Positioned.fill` layer in a Stack, behind the
// things that take their own gestures (bars, key handles). Dragging on empty
// space draws the box; on release [onSelect] gets the box's rectangle in local
// coordinates and the owner decides what fell inside it. A plain click calls
// [onClear] — a selection box around nothing means "select nothing" everywhere.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'controls.dart';

class MarqueeSelect extends StatefulWidget {
  /// The finished box, in this widget's own coordinates, and whether the drag
  /// was **additive** — `Shift` or `Ctrl` held when it started (K-500 §2.1).
  ///
  /// Read at the drag's *start* rather than at its release, because that is
  /// when the gesture was decided: letting go of Shift half way through a box
  /// should not turn an adding drag into a replacing one, and neither should
  /// pressing it.
  final void Function(Rect rect, bool additive) onSelect;

  /// A plain click on the background: clear the owner's selection.
  final VoidCallback onClear;

  /// When set, a click reports *where* it landed instead of calling
  /// [onClear] — for owners whose background click means more than "clear"
  /// (the graph editor's Ctrl+click plants a key on the curve there).
  final void Function(Offset local)? onTapAt;

  const MarqueeSelect({
    super.key,
    required this.onSelect,
    required this.onClear,
    this.onTapAt,
  });

  @override
  State<MarqueeSelect> createState() => _MarqueeSelectState();
}

class _MarqueeSelectState extends State<MarqueeSelect> {
  Offset? _from;
  Offset? _to;

  /// Whether the modifier that adds to the standing selection was down when
  /// this drag began.
  bool _additive = false;

  void _finish() {
    final from = _from;
    final to = _to;
    final additive = _additive;
    setState(() {
      _from = null;
      _to = null;
    });
    if (from == null || to == null) return;
    widget.onSelect(Rect.fromPoints(from, to), additive);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            // **Everything but the trackpad drags a box** — see [dragDevices]
            // for why the trackpad's two-finger pan is left to the scrollable
            // underneath.
            supportedDevices: dragDevices,
            onTap: widget.onTapAt == null ? widget.onClear : null,
            onTapUp: widget.onTapAt == null
                ? null
                : (d) => widget.onTapAt!(d.localPosition),
            // Down, not start: a pan's start position is where the slop was
            // exceeded, which would eat the box's first corner and whatever
            // sat nearest it.
            onPanDown: (d) {
              _from = d.localPosition;
              final keys = HardwareKeyboard.instance;
              _additive = keys.isShiftPressed ||
                  keys.isControlPressed ||
                  keys.isMetaPressed;
            },
            onPanStart: (d) => setState(() => _to = d.localPosition),
            onPanUpdate: (d) => setState(() => _to = d.localPosition),
            onPanEnd: (_) => _finish(),
            onPanCancel: () => setState(() {
              _from = null;
              _to = null;
            }),
          ),
        ),
        if (_from != null && _to != null)
          Positioned.fromRect(
            rect: Rect.fromPoints(_from!, _to!),
            child: IgnorePointer(
              child: Container(
                decoration: BoxDecoration(
                  color: t.accent.withValues(alpha: 0.12),
                  border: Border.all(color: t.accent, width: 1),
                ),
              ),
            ),
          ),
      ],
    );
  }
}
