// The stopwatch and keyframe navigator, shared by the Transform rows and the
// effect parameter rows.
//
// One widget rather than two because the two rows differ only in *where* the
// value lives — a transform property or an effect parameter — and not at all in
// what keying it means. Each caller hands over the scalar and a way to write a
// new one; everything else is the same on both sides, which is what stops the
// two drifting into slightly different ideas of what the diamond does.
//
// **What the controls do** (docs/07 §5, matching After Effects):
//
// - **Stopwatch** turns animation on and off. Turning it on plants one key at
//   the playhead holding the value that is already there, so nothing moves.
//   Turning it off keeps the value the curve reads *at the playhead* rather than
//   snapping to the first key — which is why the sampling is done engine-side.
//   It is `animated` amber while the property is keyed and muted otherwise
//   (docs/15 §3.1), and square under Sharp (§12A.3).
// - **Previous key / next key** jump to the neighbouring keys, moving the
//   playhead.
// - **Add key** adds a key at the playhead, or removes the one already there.
//   Amber when the playhead sits on a key, muted when it does not — the set has
//   one weight and no filled variants (§5), so the state is said in colour.
//
// All four are drawn from Lumit's own icon set; the arrowheads used to
// be bare Unicode characters, which §5 forbids outright.
//
// Every one of these is a single write of the whole animation, so each is one
// undo step — the reason the frb API takes a whole `BridgeScalar` rather than
// v0's granular add/remove/shift ops, where a key drag that moved time *and*
// value cost two.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart' show iconSize;
import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'fx_section.dart';
import 'graph_maths.dart' show keyframeAmong;

/// The scalar with `value` written at `frame`: the key already there is
/// updated, or a new linear key is inserted in order. This is what typing or
/// dragging an *animated* value in the outline means (docs/07 §4.3) — on a
/// keyframe it edits that keyframe; between keyframes it plants one, exactly
/// as After Effects does. A static scalar just becomes the new value.
BridgeScalar scalarWithValueAt(
  BridgeScalar scalar,
  double value,
  CompositionReference comp,
  int frame,
) {
  if (scalar is! BridgeScalar_Keyframed) return BridgeScalar.static_(value);
  final next = <BridgeKeyframe>[];
  var replaced = false;
  for (final key in scalar.field0) {
    if (comp.frameAtTime(time: key.time) == frame) {
      next.add(BridgeKeyframe(
        time: key.time,
        value: value,
        interpIn: key.interpIn,
        interpOut: key.interpOut,
      ));
      replaced = true;
    } else {
      next.add(key);
    }
  }
  if (!replaced) {
    next
      ..add(keyframeAmong(
          scalar.field0, comp.timeOfFrame(frame: frame), value))
      ..sort((a, b) => comp
          .frameAtTime(time: a.time)
          .compareTo(comp.frameAtTime(time: b.time)));
  }
  return BridgeScalar.keyframed(next);
}

/// The scalar with **every** value multiplied by `k`, curve shape held.
///
/// This is what scaling a chained pair's other half means: each key
/// keeps its time and its interpolation, and the whole curve is the same shape
/// drawn against a stretched value axis. A bezier side's `speed` is in value
/// units per second, so it goes with the values; influences and times are the
/// other axis and are left alone. An Auto side's remembered ease is not
/// evaluated, and an expression computes its own number — both are untouched.
///
/// The same arithmetic as `scale_property` in `crates/lumit-core/src/fx/
/// builtins.rs`, which is how an earlier migration rescaled a keyed property.
BridgeScalar scaledScalar(BridgeScalar scalar, double k) => switch (scalar) {
      BridgeScalar_Static(:final field0) => BridgeScalar.static_(field0 * k),
      BridgeScalar_Keyframed(:final field0) => BridgeScalar.keyframed([
          for (final key in field0)
            BridgeKeyframe(
              time: key.time,
              value: key.value * k,
              interpIn: _scaledSide(key.interpIn, k),
              interpOut: _scaledSide(key.interpOut, k),
            ),
        ]),
      BridgeScalar_Expression() => scalar,
    };

BridgeSideInterp _scaledSide(BridgeSideInterp side, double k) => switch (side) {
      BridgeSideInterp_Bezier(:final field0) => BridgeSideInterp.bezier(
          BridgeBezierSide(
              speed: field0.speed * k, influence: field0.influence)),
      _ => side,
    };

/// A property's name as a **flat sheet** writes it (§3.2 of
/// `docs/impl/timeline-interaction.md`): the group it came out of, muted, a
/// middle dot, then the property's own name — `Transform · Opacity`.
///
/// Returned as the widgets that go *before* the name **inside a row's own
/// label**, rather than as a row of its own, so a property is one row wherever
/// it is listed. The fold-out draws no prefix — its group is the twirl it sits
/// under — and the dope sheet, which has no twirls to sit under, draws one.
List<Widget> flatGroupPrefix(LumitTheme t, String? group) => group == null
    ? const <Widget>[]
    : [
        Flexible(
          child: Text(group,
              style: t.body.copyWith(color: t.textMuted),
              overflow: TextOverflow.ellipsis),
        ),
        const SizedBox(width: 6),
        Text('·', style: t.body.copyWith(color: t.textDisabled)),
        const SizedBox(width: 6),
      ];

/// A value field for a scalar that is **keyframed**: it shows the value under
/// the playhead and an edit writes the key sitting there (docs/07 §4.3).
///
/// The drag is staged in Dart and committed exactly once, on release — which
/// is the whole point of it existing. [DragValueField] falls back to
/// `onChanged` for every tick when no `onChangeLive` is given, so a keyframed
/// drag was writing one op per pixel: the undo stack filled with a step per
/// tick and a single undo moved the value back by one hair instead of undoing
/// the gesture. A drag that plants a *new* key was worse — it planted one per
/// tick.
class KeyedValueField extends StatefulWidget {
  /// The key on the inner field, so tests and callers address it as they
  /// would an ordinary value field.
  final Key fieldKey;

  /// The document's value at the playhead.
  final double value;
  final double min;
  final double max;
  final double speed;
  final int decimals;
  final String? suffix;

  /// The finished edit: a released drag, or a typed value. Called once.
  final ValueChanged<double> onCommit;

  /// Each tick of a drag, if the caller wants to show it. A keyed drag stages
  /// in Dart and commits once, which left the picture standing still
  /// until the release — the same complaint the graph editor's drags drew, for
  /// the same reason. Optional: a caller with nothing to preview passes
  /// nothing and behaves exactly as before.
  final ValueChanged<double>? onLive;

  /// The gesture beginning, before any value has moved. A caller that keys on
  /// drag-start uses it: the property is animated, the playhead is
  /// between keys, and the drag is about to edit *something* — so a key holding
  /// the value already there is planted, and nothing moves until the pointer
  /// does.
  final VoidCallback? onStart;

  const KeyedValueField({
    super.key,
    required this.fieldKey,
    required this.value,
    required this.onCommit,
    this.onLive,
    this.onStart,
    this.min = -1000000,
    this.max = 1000000,
    this.speed = 1,
    this.decimals = 2,
    this.suffix,
  });

  @override
  State<KeyedValueField> createState() => _KeyedValueFieldState();
}

class _KeyedValueFieldState extends State<KeyedValueField> {
  /// The value under the pointer mid-drag; null when nothing is in flight.
  double? _staged;

  void _commit(num value) {
    setState(() => _staged = null);
    widget.onCommit(value.toDouble());
  }

  @override
  Widget build(BuildContext context) => DragValueField(
        key: widget.fieldKey,
        value: _staged ?? widget.value,
        // This field exists only for a keyframed scalar, so its number rests
        // in `animated` (§3.1) — the well is where a keyed property says so.
        keyed: true,
        min: widget.min,
        max: widget.max,
        speed: widget.speed,
        decimals: widget.decimals,
        suffix: widget.suffix,
        // Typed, reset and pasted values are already one-shot edits.
        onChanged: _commit,
        onChangeStart: () {
          setState(() => _staged = widget.value);
          widget.onStart?.call();
        },
        // A tick moves the number on screen, and shows it if the caller can.
        onChangeLive: (v) {
          setState(() => _staged = v.toDouble());
          widget.onLive?.call(v.toDouble());
        },
        onChangeEnd: _commit,
        onDragCancel: () => setState(() => _staged = null),
      );
}

class KeyframeControlsFrb extends StatelessWidget {
  /// The animations this control covers — one for a single value, several for a
  /// row that spans axes (Position's x and y).
  ///
  /// A multi-axis row keys its axes *together*: they are separate properties in
  /// the model, which is what makes a per-axis curve possible, but one stopwatch
  /// covering them has to act on all of them or it is lying about what it
  /// controls.
  final List<BridgeScalar> scalars;

  /// Commit a new animation for each — in ONE undo step. A caller spanning
  /// several properties must batch them; two ops for one click is what the
  /// whole-value shape exists to avoid.
  final ValueChanged<List<BridgeScalar>> onWrite;

  /// The comp, for turning frames into the exact rational times keys carry.
  final CompositionReference comp;

  final int playheadFrame;
  final ValueChanged<int> onSeek;

  /// Distinguishes this row's buttons in a panel full of them.
  final String rowKey;

  /// Lay the controls out on the Effect controls panel's **fixed columns**:
  /// the stopwatch in a column of [fxStopwatchColumn], the navigator
  /// in a slot of [fxKeyNavColumn] that keeps its width whether or not there is
  /// anything in it. That is what stops a label moving sideways the moment its
  /// stopwatch is switched on.
  ///
  /// The Timeline's fold-out passes false, the default: its lanes answer to the
  /// render-switch column group and have no room to reserve a navigator on
  /// every row.
  final bool fixedColumns;

  const KeyframeControlsFrb({
    super.key,
    required this.scalars,
    required this.onWrite,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.rowKey,
    this.fixedColumns = false,
  });

  /// The row's first animation, which is what the navigator reads.
  ///
  /// A multi-axis row keys every axis at the same times, so its axes agree about
  /// *where* the keys are even when they disagree about the values. Reading one
  /// is therefore enough to draw the diamonds and find the neighbours.
  BridgeScalar get _lead => scalars.first;

  List<BridgeKeyframe> _keysOf(BridgeScalar scalar) => switch (scalar) {
        BridgeScalar_Keyframed(:final field0) => field0,
        BridgeScalar_Static() => const [],
        BridgeScalar_Expression() => const [],
      };

  List<BridgeKeyframe> get _keys => _keysOf(_lead);

  /// True when *any* axis is animated. A row half-animated by the graph editor
  /// still shows its stopwatch lit, because turning it off is then the useful
  /// action.
  bool get _animated => scalars.any((s) => _keysOf(s).isNotEmpty);

  /// What each axis reads at `frame` — what a new key on it takes, so adding
  /// one never moves anything.
  List<double> _valuesNow(int frame) {
    final time = timeOfFrame(comp, frame);
    return [for (final s in scalars) sampledScalar(s, time)];
  }

  /// The key sitting exactly on `frame`, if there is one.
  ///
  /// Compared by *frame*, not by rational equality: a key placed at frame 24
  /// and the playhead at frame 24 are the same key to the user even if some
  /// other route stored an unreduced time.
  BridgeKeyframe? _keyAt(int frame) {
    for (final key in _keys) {
      if (frameAtTime(comp, key.time) == frame) return key;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    // The live playhead, not the one captured when the panel last drew: the
    // diamond fills exactly while the playhead sits on a key, and hollows the
    // moment it scrubs away (docs/07 §4.3).
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    // **A still row does not listen.** With no keys there is no diamond to
    // fill, no neighbour to step to and no curve to sample, so every frame
    // draws the same controls — and a row that rebuilt them anyway was the bulk
    // of what a scrub cost in Effect controls. The handlers below still read
    // the *live* playhead, because pressing the stopwatch keys where the
    // playhead is now, not where it was when this row last drew.
    if (!_animated) return _build(context, playhead.value, playhead);
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) => _build(context, frame, playhead),
    );
  }

  Widget _build(
      BuildContext context, int frame, ValueListenable<int> playhead) {
    final t = ThemeScope.of(context).theme;
    final onKey = _keyAt(frame) != null;
    final previous = _neighbour(frame, before: true);
    final next = _neighbour(frame, before: false);

    final stopwatch = LumitTooltip(
      message: _animated ? l10n.tipStopAnimating : l10n.tipAnimate,
      child: _button(
        keyName: 'kf-stopwatch-$rowKey',
        // The one place the stopwatch has colour of its own: `animated` says
        // the property is keyed (§3.1's closed job list), never the accent,
        // which the redesign spends on the filled action and the playhead.
        child: LumitIcon(LumitIcons.stopwatch,
            size: iconSize, colour: _animated ? t.animated : t.textMuted),
        onPressed: () => _toggleAnimated(playhead.value),
      ),
    );

    final navigator = <Widget>[
      _button(
        keyName: 'kf-prev-$rowKey',
        enabled: previous != null,
        child: LumitIcon(LumitIcons.previousKey,
            size: iconSize,
            colour: previous == null ? t.textDisabled : t.textMuted),
        onPressed: () => _seekTo(previous),
      ),
      LumitTooltip(
        message: onKey ? l10n.tipRemoveKeyframe : l10n.tipAddKeyframe,
        child: _button(
          keyName: 'kf-toggle-$rowKey',
          child: LumitIcon(LumitIcons.addKey,
              size: iconSize, colour: onKey ? t.animated : t.textMuted),
          onPressed: () => _toggleKeyHere(playhead.value),
        ),
      ),
      _button(
        keyName: 'kf-next-$rowKey',
        enabled: next != null,
        child: LumitIcon(LumitIcons.nextKey,
            size: iconSize,
            colour: next == null ? t.textDisabled : t.textMuted),
        onPressed: () => _seekTo(next),
      ),
    ];

    if (!fixedColumns) {
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [stopwatch, if (_animated) ...navigator],
      );
    }

    // The two reserved columns. The navigator's slot keeps its width while the
    // property is static, so switching the stopwatch on adds three buttons
    // without moving the label a pixel.
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(width: fxStopwatchColumn, child: stopwatch),
        SizedBox(
          width: fxKeyNavColumn,
          child: _animated
              ? Row(mainAxisSize: MainAxisSize.min, children: navigator)
              : const SizedBox.shrink(),
        ),
      ],
    );
  }

  Widget _button({
    required String keyName,
    required Widget child,
    required VoidCallback onPressed,
    bool enabled = true,
  }) =>
      HouseButton(
        key: ValueKey<String>(keyName),
        frameless: true,
        small: true,
        // No vertical padding: the icon is 16 px and an Effect controls row
        // gives its controls 18 (`fxRowHeight`), which the border then fills.
        // Padding on top of that spilled the icon out of the row.
        //
        // Horizontally the two layouts differ. On the fixed columns it is
        // nothing at all: the button's own always-reserved 1px edge already
        // brings a 16px glyph to the 18px those columns are measured in, and
        // any more would burst them. The Timeline's fold-out is not measured in
        // those columns, so it keeps the 3px it always had — losing it shrank
        // its buttons and made them harder to hit.
        padding: EdgeInsets.symmetric(horizontal: fixedColumns ? 0 : 3),
        onPressed: enabled ? onPressed : null,
        child: child,
      );

  /// Animation on: one key at the playhead holding what is already there.
  /// Animation off: the value the curve reads at the playhead, so turning it off
  /// leaves the picture where it is rather than jumping to the first key.
  void _toggleAnimated(int frame) {
    final values = _valuesNow(frame);
    if (_animated) {
      onWrite([for (final v in values) BridgeScalar.static_(v)]);
      return;
    }
    onWrite([
      for (final v in values)
        BridgeScalar.keyframed([_newKeyAt(const [], frame, v)])
    ]);
  }

  /// Add a key at the playhead, or remove the one there.
  ///
  /// Removing the last key does not leave an empty curve — an animation with no
  /// keys is not a curve anything can evaluate — so it falls back to a static
  /// value holding what that key held.
  void _toggleKeyHere(int frame) {
    final removing = _keyAt(frame) != null;
    final values = _valuesNow(frame);
    final next = <BridgeScalar>[];

    for (var axis = 0; axis < scalars.length; axis++) {
      final keys = _keysOf(scalars[axis]);
      if (removing) {
        final rest = [
          for (final k in keys)
            if (comp.frameAtTime(time: k.time) != frame) k,
        ];
        // A curve with no keys is not something the engine can evaluate, so the
        // last one removed leaves a static value holding what it held.
        next.add(rest.isEmpty
            ? BridgeScalar.static_(values[axis])
            : BridgeScalar.keyframed(rest));
        continue;
      }
      // Keys must stay strictly ascending in time — the engine enforces it on
      // the way in, so this inserts in order rather than appending and hoping.
      final added = [...keys, _newKeyAt(keys, frame, values[axis])]..sort((a, b) =>
          comp
              .frameAtTime(time: a.time)
              .compareTo(comp.frameAtTime(time: b.time)));
      next.add(BridgeScalar.keyframed(added));
    }
    onWrite(next);
  }

  /// A key planted here takes after the keys it lands between (M19), so a
  /// stopwatch pressed inside a held run does not quietly turn it linear.
  BridgeKeyframe _newKeyAt(List<BridgeKeyframe> keys, int frame, double value) =>
      keyframeAmong(keys, comp.timeOfFrame(frame: frame), value);

  /// The nearest key strictly before or after `frame`.
  BridgeKeyframe? _neighbour(int frame, {required bool before}) {
    BridgeKeyframe? best;
    int? bestFrame;
    for (final key in _keys) {
      final at = frameAtTime(comp, key.time);
      if (before ? at >= frame : at <= frame) continue;
      if (bestFrame == null || (before ? at > bestFrame : at < bestFrame)) {
        best = key;
        bestFrame = at;
      }
    }
    return best;
  }

  void _seekTo(BridgeKeyframe? key) {
    if (key == null) return;
    onSeek(comp.frameAtTime(time: key.time));
  }
}

/// The stopwatch and ◄ ◆ ► for a mask's **shape**.
///
/// The same controls as [KeyframeControlsFrb] and deliberately in the same
/// file, so the two cannot drift into different ideas of what a diamond does.
/// What differs is only what a key *holds*: a whole path rather than a number,
/// which is why this one writes through the engine's own path-key ops instead
/// of sending an animation. There is no value to plot, so the lane shows
/// diamonds and no curve, and the row has no field.
class PathKeyframesFrb extends StatelessWidget {
  /// This row's shape keys, as they came across with the mask or the
  /// shape item they belong to.
  final List<BridgeKeyframe> keys;

  /// Plant or take away a key at this composition time — the diamond.
  final void Function(BridgeRational time) onToggleKey;

  /// Stop the shape animating, keeping the shape shown at this time — the
  /// stopwatch turning off.
  final void Function(BridgeRational time) onClear;

  /// What the buttons' widget keys are built from, so a test can find the row
  /// it means.
  final String rowKey;

  final CompositionReference comp;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const PathKeyframesFrb({
    super.key,
    required this.keys,
    required this.onToggleKey,
    required this.onClear,
    required this.rowKey,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  bool get _animated => keys.isNotEmpty;

  /// The frames the shape is keyed on.
  ///
  /// **Through the shared memory, not straight at the engine**. This
  /// getter is read inside the build below, so what it cost was a bridge call
  /// per key per rebuild — the traffic `bridge_call_budget_test` exists to
  /// keep out of build paths, and the sibling control four methods up was
  /// already going through the memo.
  List<int> get _frames => [for (final k in keys) frameAtTime(comp, k.time)];

  int? _neighbour(int frame, {required bool before}) {
    int? best;
    for (final f in _frames) {
      if (before ? f >= frame : f <= frame) continue;
      if (best == null || (before ? f > best : f < best)) best = f;
    }
    return best;
  }

  void _toggleKeyHere(int frame) {
    try {
      onToggleKey(comp.timeOfFrame(frame: frame));
      onChanged();
    } catch (_) {
      // The mask or the item went away between the draw and the click.
    }
  }

  /// On: one key at the playhead holding the shape already showing, so nothing
  /// moves. Off: the shape the playhead is over is kept as the static path.
  void _toggleAnimated(int frame) {
    try {
      final time = comp.timeOfFrame(frame: frame);
      if (_animated) {
        onClear(time);
      } else {
        onToggleKey(time);
      }
      onChanged();
    } catch (_) {
      // As above.
    }
  }

  @override
  Widget build(BuildContext context) {
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    // A still path draws the same row at every frame — see the note on
    // [KeyframeControlsFrb.build].
    if (!_animated) return _build(context, playhead.value, playhead);
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) => _build(context, frame, playhead),
    );
  }

  Widget _build(
      BuildContext context, int frame, ValueListenable<int> playhead) {
    final t = ThemeScope.of(context).theme;
    final onKey = _frames.contains(frame);
    Widget button({
      required String keyName,
      required Widget child,
      required VoidCallback onPressed,
      bool enabled = true,
    }) =>
        HouseButton(
          key: ValueKey<String>(keyName),
          frameless: true,
          small: true,
          padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 2),
          onPressed: enabled ? onPressed : null,
          child: child,
        );

    final previous = _neighbour(frame, before: true);
    final next = _neighbour(frame, before: false);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        LumitTooltip(
          message: _animated ? l10n.tipStopAnimating : l10n.tipAnimate,
          child: button(
            keyName: 'kf-stopwatch-$rowKey',
            child: LumitIcon(LumitIcons.stopwatch,
                size: iconSize, colour: _animated ? t.animated : t.textMuted),
            onPressed: () => _toggleAnimated(playhead.value),
          ),
        ),
        if (_animated) ...[
          button(
            keyName: 'kf-prev-$rowKey',
            enabled: previous != null,
            child: LumitIcon(LumitIcons.previousKey,
                size: iconSize,
                colour: previous == null ? t.textDisabled : t.textMuted),
            onPressed: () => onSeek(previous!),
          ),
          LumitTooltip(
            message: onKey ? l10n.tipRemoveKeyframe : l10n.tipAddKeyframe,
            child: button(
              keyName: 'kf-toggle-$rowKey',
              child: LumitIcon(LumitIcons.addKey,
                  size: iconSize, colour: onKey ? t.animated : t.textMuted),
              onPressed: () => _toggleKeyHere(playhead.value),
            ),
          ),
          button(
            keyName: 'kf-next-$rowKey',
            enabled: next != null,
            child: LumitIcon(LumitIcons.nextKey,
                size: iconSize,
                colour: next == null ? t.textDisabled : t.textMuted),
            onPressed: () => onSeek(next!),
          ),
        ],
      ],
    );
  }
}
