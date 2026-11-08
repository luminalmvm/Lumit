// A parameter's row on an open box: the name, muted, and the value at the
// playhead, bright. The box says what a value is; the Node panel's row is
// where it is set, so a box stays the size of its facts.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart' show ThemeScope;
import 'effect_param_row_frb.dart' show scalarValueAt, unitRiderText;
import 'graph_panel.dart' show graphRowInset;

class GraphValueRow extends StatelessWidget {
  final BridgeParamInfo param;
  final BridgeEffectValue? value;
  final CompositionReference comp;
  final int frame;

  /// False where another control has taken the row over.
  final bool enabled;

  const GraphValueRow({
    super.key,
    required this.param,
    required this.value,
    required this.comp,
    required this.frame,
    this.enabled = true,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final bright = enabled ? t.textPrimary : t.textDisabled;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: graphRowInset),
      child: Row(
        children: [
          Expanded(
            child: Text(engineLabel(param.label),
                style: t.small
                    .copyWith(color: enabled ? t.textMuted : t.textDisabled),
                overflow: TextOverflow.ellipsis),
          ),
          if (_value(t, bright) case final value?) ...[
            const SizedBox(width: 8),
            value,
          ],
        ],
      ),
    );
  }

  /// The value as the box reads it: a number with its unit, a point, a
  /// swatch, a tick, a choice's word. A keyed number wears the keyed colour.
  /// Null for a kind with nothing short to say, which draws the name alone.
  Widget? _value(LumitTheme t, Color bright) {
    final mono = t.mono.copyWith(fontSize: 10, color: bright);
    double? at(BridgeScalar scalar) => scalarValueAt(scalar, comp, frame);
    switch (value) {
      case BridgeEffectValue_Float(:final field0):
        final v = at(field0);
        if (v == null) return null;
        final whole = param.kind is BridgeParamKind_Int;
        final unit = unitRiderText(param.unit);
        final keyed = field0 is BridgeScalar_Keyframed;
        return Text(
          '${whole ? v.round() : v.toStringAsFixed(2)}${unit == null ? '' : ' $unit'}',
          style: keyed ? mono.copyWith(color: t.animated) : mono,
        );
      case BridgeEffectValue_Point(:final field0):
        final x = at(field0.x);
        final y = at(field0.y);
        if (x == null || y == null) return null;
        return Text('${x.toStringAsFixed(1)}, ${y.toStringAsFixed(1)}',
            style: mono);
      case BridgeEffectValue_Colour(:final field0):
        final r = at(field0.r);
        final g = at(field0.g);
        final b = at(field0.b);
        final a = at(field0.a);
        if (r == null || g == null || b == null || a == null) return null;
        return Container(
          width: 12,
          height: 12,
          decoration: BoxDecoration(
            color: Color.from(
              alpha: a.clamp(0, 1),
              red: r.clamp(0, 1),
              green: g.clamp(0, 1),
              blue: b.clamp(0, 1),
            ),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius - 1),
            border: Border.all(color: t.hairline),
          ),
        );
      case BridgeEffectValue_Bool(:final field0):
        return field0
            ? glyph.LumitIcon(LumitIcons.tick, size: 10, colour: bright)
            : null;
      case BridgeEffectValue_Choice(:final field0):
        if (param.kind case BridgeParamKind_Choice(:final options)
            when field0 >= 0 && field0 < options.length) {
          return Flexible(
            child: Text(engineLabel(options[field0]),
                style: t.small.copyWith(color: bright),
                overflow: TextOverflow.ellipsis),
          );
        }
        return null;
      case BridgeEffectValue_Text(:final field0):
        return Flexible(
          child: Text(field0,
              style: t.small.copyWith(color: bright),
              maxLines: 1,
              overflow: TextOverflow.ellipsis),
        );
      default:
        return null;
    }
  }
}
