// The Ease popover: the small box that eases a block of selected keyframes
// where they stand (the approved Keys drawing; docs/15 §12A.1a).
//
// In plain terms: pick several keyframes and this is the quick way to shape
// how the movement between them runs. Choose a curve by name, adjust how far
// each end of it reaches, optionally fan the rows out so they arrive one after
// another, and press Apply. It is deliberately small — four lines and two
// buttons — because it is opened *on* a selection and is meant to be gone
// again in a moment.
//
// **Not a second easing editor.** The shapes are the same [EasingCurve]
// presets the Easing panel draws, and Apply lands through the same
// `applyEasingToSelection` the panel's does; Open graph hands the same
// selection to the graph editor, where the shape can be drawn by hand instead
// of chosen. What this adds is the two things a dope sheet wants and a curve
// box cannot say: the reach of each side as a plain percentage, and a stagger.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'easing_curve.dart';
import 'easing_editor.dart' show easingPresetName;
import 'key_block.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// The popover's width, from the drawing's 190px face.
const double easePopoverWidth = 190;

/// The label column, so the four rows' controls line up under one another —
/// the drawing's 56.
const double _labelWidth = 56;

/// A value well's width in this box: the drawing's 44px face.
const double _wellWidth = 44;

/// What one press of Apply asks for: a shape, and how far apart to spread the
/// rows it lands on.
class EaseRequest {
  final EasingCurve curve;

  /// Frames between one row's keys and the next. Zero — the resting value —
  /// leaves every row where it is, so a stagger is something you ask for
  /// rather than something you have to switch off.
  final double stagger;
  final StaggerOrder order;

  const EaseRequest({
    required this.curve,
    required this.stagger,
    required this.order,
  });
}

/// Open the Ease popover at [position], anchored to the block it acts on.
///
/// [count] is how many keys the block holds, which the header says back so the
/// box is plainly about the selection rather than about the panel. [onApply]
/// is called on each press of Apply and the box stays up — easing is a "try it,
/// nudge it, try it again" job — and [onOpenGraph] closes it and hands the same
/// keys to the graph editor.
Future<void> showEasePopover({
  required BuildContext context,
  required Offset position,
  required int count,
  required ValueChanged<EaseRequest> onApply,
  required VoidCallback onOpenGraph,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _EasePopover(
        count: count,
        onApply: onApply,
        onOpenGraph: () {
          close(null);
          onOpenGraph();
        },
      ),
    );

class _EasePopover extends StatefulWidget {
  final int count;
  final ValueChanged<EaseRequest> onApply;
  final VoidCallback onOpenGraph;

  const _EasePopover({
    required this.count,
    required this.onApply,
    required this.onOpenGraph,
  });

  @override
  State<_EasePopover> createState() => _EasePopoverState();
}

class _EasePopoverState extends State<_EasePopover> {
  /// It opens on the gentlest preset — the F9 easy ease, which is what the
  /// overwhelming majority of these presses want.
  EasingPreset _preset = easingPresets.first;

  /// The two reaches, as the percentages the drawing shows. Held apart from
  /// the preset because they are edited after it: picking a curve loads its
  /// reaches, and nudging a number leaves the curve's name behind.
  late double _out = _preset.curve.x1;
  late double _into = 1 - _preset.curve.x2;

  double _stagger = 0;
  StaggerOrder _order = StaggerOrder.topDown;

  /// The shape the two numbers describe: the preset's *vertical* control
  /// points — what makes an overshoot overshoot — with the reaches the fields
  /// hold.
  ///
  /// Splitting the four numbers this way is what lets Influence be two plain
  /// percentages rather than four coordinates: the y values say what kind of
  /// ease this is, and the x values say how far into the span it reaches, which
  /// is exactly the sense "influence" carries on a keyframe (docs/impl/
  /// keyframe-eval.md §1).
  EasingCurve get _curve => EasingCurve(
        _out,
        _preset.curve.y1,
        1 - _into,
        _preset.curve.y2,
      );

  void _pickPreset(EasingPreset preset) => setState(() {
        _preset = preset;
        _out = preset.curve.x1;
        _into = 1 - preset.curve.x2;
      });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Its own surface rather than [FloatSurface]: this box has a header strip
    // that runs to its edges, and a menu surface's uniform 6px inset would put
    // a margin round it. The face, the hairline and the shadow are the same
    // three the drawing gives every floating card.
    return Container(
      width: easePopoverWidth,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border.all(color: t.hairline),
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        boxShadow: t.floatShadow,
      ),
      clipBehavior: Clip.antiAlias,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The header: what this box is, and what it has hold of. A raised
          // strip, the way every small floating surface's title is.
          Container(
            height: t.density.laneRow,
            decoration: BoxDecoration(
              color: t.surface2,
              border: Border(bottom: BorderSide(color: t.hairline)),
            ),
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              children: [
                Text(l10n.easeBlockTitle.toUpperCase(), style: t.kickerOn),
                const Spacer(),
                Text(
                  l10n.easeKeyCount(widget.count),
                  key: const ValueKey('ease-count'),
                  // Sentence case and half the tracking: this counts, it does
                  // not label, so it wears the kicker's size without its shout.
                  style: t.kicker.copyWith(letterSpacing: 0.54),
                ),
              ],
            ),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _row(t, l10n.easeCurve, _curvePicker(t)),
                const SizedBox(height: 6),
                _row(t, l10n.easeInfluence, _influence(t)),
                const SizedBox(height: 6),
                _row(t, l10n.easeStagger, _staggerRow(t)),
                const SizedBox(height: 8),
                _buttons(t),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// One labelled line: the name in the fixed column, the control filling what
  /// is left.
  Widget _row(LumitTheme t, String label, Widget control) => Row(
        children: [
          SizedBox(
            width: _labelWidth,
            child: Text(label,
                style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          const SizedBox(width: 8),
          Expanded(child: control),
        ],
      );

  /// The curve, by name. A picker rather than the Easing panel's row of chips:
  /// there is one line for it here, and a name that says what the shape does is
  /// the whole of what this box needs to say about it.
  Widget _curvePicker(LumitTheme t) => Builder(
        builder: (fieldContext) => HouseButton(
          key: const ValueKey('ease-curve'),
          small: true,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          onPressed: () async {
            final box = fieldContext.findRenderObject();
            if (box is! RenderBox) return;
            final picked = await showMenuAt<EasingPreset>(
              context: fieldContext,
              position: box.localToGlobal(Offset.zero),
              width: easePopoverWidth - 20,
              rows: (close) => [
                for (final preset in easingPresets)
                  MenuRow(
                    key: ValueKey<String>('ease-curve-${preset.id}'),
                    onPressed: () => close(preset),
                    child: Text(easingPresetName(preset.id)),
                  ),
              ],
            );
            if (picked != null) _pickPreset(picked);
          },
          child: Align(
            alignment: Alignment.centerLeft,
            child: Text(easingPresetName(_preset.id),
                style: t.body.copyWith(color: t.textPrimary)),
          ),
        ),
      );

  /// The two reaches as percentages: out of the first key, and into the last.
  ///
  /// Clamped away from 0 and 100 by [EasingCurve] itself, so a field driven to
  /// either end still stores a shape the engine will evaluate (docs/impl/
  /// keyframe-eval.md §1 keeps the curve x-monotone).
  Widget _influence(LumitTheme t) => Row(
        children: [
          Flexible(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: _wellWidth),
              child: DragValueField(
                key: const ValueKey('ease-influence-out'),
                value: (_out * 100).round(),
                min: 1,
                max: 100,
                onChanged: (v) => setState(() => _out = v / 100),
              ),
            ),
          ),
          const SizedBox(width: 6),
          Flexible(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: _wellWidth),
              child: DragValueField(
                key: const ValueKey('ease-influence-in'),
                value: (_into * 100).round(),
                min: 1,
                max: 100,
                onChanged: (v) => setState(() => _into = v / 100),
              ),
            ),
          ),
          const SizedBox(width: 6),
          Text('%', style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
        ],
      );

  /// The stagger: how many frames apart the rows arrive, and from which end.
  Widget _staggerRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: _wellWidth,
            child: DragValueField(
              key: const ValueKey('ease-stagger'),
              value: _stagger,
              min: 0,
              max: 240,
              onChanged: (v) => setState(() => _stagger = v.toDouble()),
            ),
          ),
          const SizedBox(width: 6),
          Text('f', style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
          const SizedBox(width: 6),
          Expanded(
            child: HouseButton(
              key: const ValueKey('ease-stagger-order'),
              small: true,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              onPressed: () => setState(() => _order =
                  _order == StaggerOrder.topDown
                      ? StaggerOrder.bottomUp
                      : StaggerOrder.topDown),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(
                  _order == StaggerOrder.topDown
                      ? l10n.easeStaggerTopDown
                      : l10n.easeStaggerBottomUp,
                  style: t.body,
                ),
              ),
            ),
          ),
        ],
      );

  /// The way out and the way on. Open graph is a kicker, not a button: it goes
  /// somewhere rather than doing something, which is the same grammar the
  /// bottom bars use.
  Widget _buttons(LumitTheme t) => Row(
        mainAxisAlignment: MainAxisAlignment.end,
        children: [
          Flexible(
            child: HouseButton(
              key: const ValueKey('ease-open-graph'),
              small: true,
              frameless: true,
              padding: const EdgeInsets.symmetric(horizontal: 4),
              onPressed: widget.onOpenGraph,
              child: Text(l10n.easeOpenGraph,
                  style: t.kicker.copyWith(letterSpacing: 0.54),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
            ),
          ),
          const SizedBox(width: 6),
          HouseButton(
            key: const ValueKey('ease-apply'),
            small: true,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            onPressed: () => widget.onApply(EaseRequest(
              curve: _curve,
              stagger: _stagger,
              order: _order,
            )),
            child:
                Text(l10n.apply, style: t.body.copyWith(color: t.textPrimary)),
          ),
        ],
      );
}
