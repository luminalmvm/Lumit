// The Timeline's puppet rows: a pin, and each of its animatable numbers
// (K-704, docs/impl/puppet.md §5).
//
// **In plain terms.** A layer with pins in it grows a **Puppet** heading in the
// Timeline, the way a masked layer grows a Masks one, with a row per pin under
// it and a row per number under each pin. Everything a puppet does over time is
// therefore edited where everything else is: the same stopwatch, the same
// diamonds, the same graph. There is no puppet timeline, because a pin is an
// ordinary animated property and never needed one.
//
// A pin is renamed inline like a mask, and deleted from the same right-click
// menu. The **extent** — how far a starch, overlap or bend pin reaches — sits in
// the value column on the pin's own row, where a mask keeps its opacity, because
// it is the one number on a pin that cannot be animated: which vertices a pin
// reaches is a fact about the mesh at rest, and that is what lets the solver
// factor its matrices once.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_mask_rows_frb.dart' show ItemOpacityRow;

/// The icon each kind of pin wears in the Timeline, so a list of pins can be
/// read at a glance — the same glyphs the four tools carry on the strip.
LumitIcon puppetPinIcon(BridgePuppetPinKind kind) => switch (kind) {
      BridgePuppetPinKind.position => LumitIcon.puppetPin,
      BridgePuppetPinKind.starch => LumitIcon.puppetStarch,
      BridgePuppetPinKind.overlap => LumitIcon.puppetOverlap,
      BridgePuppetPinKind.bend => LumitIcon.puppetBend,
    };

/// One pin's own row: its name, and how far it reaches.
class PuppetPinRow extends StatelessWidget {
  final LayerReference layer;
  final BridgePuppetPin pin;
  final ValueColumn valueColumn;
  final VoidCallback onChanged;

  const PuppetPinRow({
    super.key,
    required this.layer,
    required this.pin,
    required this.valueColumn,
    required this.onChanged,
  });

  void _write(BridgePuppetPin next) {
    try {
      layer.setPuppetPin(pin: next);
      onChanged();
    } catch (_) {
      // The pin or its layer went away between the draw and the click.
    }
  }

  @override
  Widget build(BuildContext context) => ItemOpacityRow(
        icon: puppetPinIcon(pin.kind),
        name: pin.name,
        keyPrefix: 'tl-puppet-pin',
        id: pin.id.toString(),
        // A position pin reaches nothing — it takes hold of one spot — so its
        // value column is empty rather than showing a number that governs
        // nothing.
        opacity: null,
        valueColumn: valueColumn,
        onRename: (name) => _write(puppetPinCopy(pin, name: name)),
        onDelete: () {
          try {
            layer.deletePuppetPin(id: pin.id);
            onChanged();
          } catch (_) {
            // Already gone, which is what was asked for.
          }
        },
        deleteLabel: l10n.deletePuppetPin,
        extra: pin.kind == BridgePuppetPinKind.position
            ? null
            : _ExtentField(pin: pin, onCommit: _write),
      );
}

/// A starch, overlap or bend pin's **extent**: how far it reaches, in the rest
/// mesh's own pixels.
///
/// Not animatable, so it has no stopwatch and no lane; it is a plain field on
/// the pin's own row, staged during the drag and committed once on release, so
/// one drag is one op and one undo step.
class _ExtentField extends StatefulWidget {
  final BridgePuppetPin pin;
  final void Function(BridgePuppetPin) onCommit;

  const _ExtentField({required this.pin, required this.onCommit});

  @override
  State<_ExtentField> createState() => _ExtentFieldState();
}

class _ExtentFieldState extends State<_ExtentField> {
  double? _staged;

  void _commit(num v) {
    setState(() => _staged = null);
    widget.onCommit(puppetPinCopy(widget.pin, extent: v.toDouble()));
  }

  @override
  Widget build(BuildContext context) => SizedBox(
        width: 72,
        child: LumitTooltip(
          message: l10n.tipPuppetExtent,
          child: DragValueField(
            key: ValueKey<String>('tl-puppet-extent-${widget.pin.id}'),
            value: _staged ?? widget.pin.extent,
            min: 1,
            max: 10000,
            suffix: ' px',
            onChanged: _commit,
            onChangeLive: (v) => setState(() => _staged = v.toDouble()),
            onChangeEnd: _commit,
            onDragCancel: () => setState(() => _staged = null),
          ),
        ),
      );
}

/// One of a pin's animatable numbers — where it stands, how much it stiffens,
/// the turn and the size it makes.
///
/// The same shape as the mask and stroke value rows beside it, and for the same
/// reasons: the value is staged while it is dragged, the release commits one op,
/// and an edit on an animated one lands on the key under the playhead rather
/// than flattening the curve.
///
/// **No live preview while it drags**, like the text animator rows: a puppet has
/// no preview call of its own, so the picture catches up when the drag is let
/// go. The op, the undo step and the committed value are the same either way.
class PuppetPinValueRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final BridgePuppetPin pin;
  final PuppetValue value;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const PuppetPinValueRow({
    super.key,
    required this.comp,
    required this.layer,
    required this.pin,
    required this.value,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<PuppetPinValueRow> createState() => _PuppetPinValueRowState();
}

class _PuppetPinValueRowState extends State<PuppetPinValueRow> {
  double? _staged;

  BridgeScalar get _scalar => puppetScalarOf(widget.pin, widget.value);

  String get _rowKey => 'tl-puppet-${widget.value.name}-${widget.pin.id}';

  /// The range and the unit each number is read in. Position is layer pixels
  /// and unbounded in practice; a starch amount runs 0..100 and an overlap's
  /// −100..100, positive in front; a bend turns in degrees and scales in per
  /// cent, with 100 the natural size.
  (double, double, String) get _range => switch (widget.value) {
        PuppetValue.positionX || PuppetValue.positionY => (-100000, 100000, ' px'),
        PuppetValue.rotation => (-100000, 100000, '°'),
        PuppetValue.scale => (1, 1000, '%'),
        PuppetValue.amount =>
          widget.pin.kind == BridgePuppetPinKind.overlap
              ? (-100, 100, '%')
              : (0, 100, '%'),
      };

  void _write(BridgeScalar v) {
    setState(() => _staged = null);
    try {
      widget.layer.setPuppetPin(
        pin: puppetPinWithScalar(widget.pin, widget.value, v),
      );
      widget.onChanged();
    } catch (_) {
      // The pin or its layer went away mid-drag.
    }
  }

  void _commitStatic(num v) => _write(BridgeScalar.static_(v.toDouble()));

  void _commitKeyed(double v) =>
      _write(scalarWithValueAt(_scalar, v, widget.comp, widget.playheadFrame));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        KeyframeControlsFrb(
          scalars: [_scalar],
          onWrite: (s) => _write(s.first),
          comp: widget.comp,
          playheadFrame: widget.playheadFrame,
          onSeek: widget.onSeek,
          rowKey: _rowKey,
        ),
        const SizedBox(width: 4),
        Expanded(
          child: Text(puppetValueLabel(widget.value),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        SizedBox(
          width: widget.valueColumn.width,
          child: Align(
            alignment: Alignment.centerLeft,
            child: SizedBox(width: 72, child: _field()),
          ),
        ),
        SizedBox(width: widget.valueColumn.rightInset),
      ],
    );
  }

  Widget _field() {
    final key = ValueKey<String>(_rowKey);
    final (min, max, suffix) = _range;
    final scalar = _scalar;
    if (scalar is! BridgeScalar_Keyframed) {
      final stored =
          _staged ?? (scalar is BridgeScalar_Static ? scalar.field0 : 0.0);
      return DragValueField(
        key: key,
        value: stored,
        min: min,
        max: max,
        decimals: 1,
        suffix: suffix,
        onChanged: _commitStatic,
        onChangeLive: (v) => setState(() => _staged = v.toDouble()),
        onChangeEnd: _commitStatic,
        onDragCancel: () => setState(() => _staged = null),
      );
    }
    // Animated: the field shows what the curve reads at the playhead, and an
    // edit writes the key there.
    return KeyedValueField(
      fieldKey: key,
      value:
          sampledScalar(scalar, timeOfFrame(widget.comp, widget.playheadFrame)),
      min: min,
      max: max,
      decimals: 1,
      suffix: suffix,
      onCommit: _commitKeyed,
    );
  }
}
