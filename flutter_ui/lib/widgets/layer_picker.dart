// The layer picker: a dropdown over a composition's layers, each one led by
// its number in the stack.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:uuid/uuid.dart';

import '../theme/theme.dart';
import 'controls.dart';

/// A layer's number as it's written beside a name: mono like the Timeline's
/// own number column, and muted so it never reads as part of the name.
TextStyle layerNumberStyle(LumitTheme t, double? size) =>
    t.mono.copyWith(fontSize: size, color: t.textMuted);

/// A layer and its number as one plain line, for a tooltip.
String numberedLayer(int number, String name) => '$number. $name';

/// Where [layer] stands in [layers], counted from 1, or null when it isn't
/// there.
int? layerNumber(List<BridgeLayerEntry> layers, UuidValue? layer) {
  if (layer == null) return null;
  final at = layers.indexWhere((e) => e.layer.internallayerId == layer);
  return at < 0 ? null : at + 1;
}

/// A dropdown that picks one of a composition's layers, or none.
///
/// The closed face and every row of the list lead with the layer's number, so
/// a layer can be found by the number its Timeline row carries and two layers
/// with one name stay apart.
class LayerPicker extends StatelessWidget {
  /// Every layer in the composition, top of the stack first. The numbers are
  /// counted from this list, so it has to be the whole composition and not
  /// only the rows on screen.
  final List<BridgeLayerEntry> layers;

  /// The layer picked, or null for none.
  final UuidValue? value;

  /// What the empty choice is called.
  final String empty;

  /// What the face reads when [value] names a layer that has gone.
  final String missing;

  /// Whether a layer is on offer. Asked when the list opens and never while
  /// drawing, so it may cross the bridge.
  final bool Function(BridgeLayerEntry) offered;

  /// What a layer is called here, when that's more than its own name.
  final String Function(BridgeLayerEntry)? name;
  final ValueChanged<UuidValue?> onChanged;

  /// The in-row face: 16 tall with a 10px label, for a picker that sits inside
  /// a Timeline row.
  final bool dense;

  const LayerPicker({
    super.key,
    required this.layers,
    required this.value,
    required this.empty,
    required this.missing,
    required this.offered,
    required this.onChanged,
    this.name,
    this.dense = false,
  });

  String _name(BridgeLayerEntry entry) => name?.call(entry) ?? entry.info.name;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final number = layerNumber(layers, value);
    final label = value == null
        ? empty
        : number == null
            ? missing
            : _name(layers[number - 1]);
    return dropdownButton(
      t: t,
      dense: dense,
      onPressed: () => _open(context),
      face: dropdownFace(
        t,
        label,
        face: number == null
            ? null
            : Flexible(
                // One line of text, so a narrow cell cuts the name short and
                // keeps the number.
                child: Builder(
                  builder: (context) => Text.rich(
                    TextSpan(children: [
                      TextSpan(
                        text: '$number',
                        style: layerNumberStyle(
                            t, DefaultTextStyle.of(context).style.fontSize),
                      ),
                      TextSpan(text: ' $label'),
                    ]),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
      ),
    );
  }

  Future<void> _open(BuildContext context) async {
    final box = context.findRenderObject()! as RenderBox;
    final origin = box.localToGlobal(Offset.zero);
    final rows = [
      for (var i = 0; i < layers.length; i++)
        if (offered(layers[i])) i,
    ];
    // Padded to the longest number, so the names start in one column.
    final digits = rows.isEmpty ? 0 : '${rows.last + 1}'.length;
    final current = GlobalKey();
    // Boxed in a record, so picking the empty choice isn't read as the list
    // being dismissed.
    final picking = showLumitPopup<(UuidValue?,)>(
      context: context,
      position: origin + Offset(0, box.size.height + 2),
      builder: (close) => FloatSurface(
        child: Builder(builder: (context) {
          final t = ThemeScope.of(context).theme;
          return IntrinsicWidth(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                MenuRow(
                  selected: value == null,
                  onPressed: () => close((null,)),
                  child: Text(empty),
                ),
                for (final i in rows)
                  MenuRow(
                    key: layers[i].layer.internallayerId == value
                        ? current
                        : null,
                    selected: layers[i].layer.internallayerId == value,
                    onPressed: () => close((layers[i].layer.internallayerId,)),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.baseline,
                      textBaseline: TextBaseline.alphabetic,
                      children: [
                        Text('${i + 1}'.padLeft(digits),
                            style: layerNumberStyle(t, t.small.fontSize)),
                        const SizedBox(width: 8),
                        Text(_name(layers[i])),
                      ],
                    ),
                  ),
              ],
            ),
          );
        }),
      ),
    );
    // A long list opens on the layer picked and not at its top.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final row = current.currentContext;
      if (row != null) Scrollable.ensureVisible(row, alignment: 0.5);
    });
    final picked = await picking;
    if (picked != null) onChanged(picked.$1);
  }
}
