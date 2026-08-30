// Layer ▸ Layer settings… (K-244, on the K-444 dialogue pattern).
//
// In plain terms: what a layer *is*, as opposed to what it is doing. Its name,
// and — on a Solid, the one kind whose picture Lumit makes rather than reads —
// the size and the colour of that picture.
//
// After Effects calls the same command Solid Settings on a solid and Layer
// Settings elsewhere; one row is enough for both, because the extra fields
// simply are not there on a kind that has no picture of its own to describe. A
// footage layer's size belongs to its file, and a text layer's to its words:
// neither is a number this dialogue has any business writing.
//
// It edits **one** layer — the primary selection. Renaming is K-523's standing
// exception to "an action runs on all of them", and every other field here is
// that layer's own identity too.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/solid.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';

/// Show the layer's settings. Completes with whether anything was applied, so
/// the caller can redraw only when there is something to redraw.
Future<bool> showLayerSettingsFrb({
  required BuildContext context,
  required LayerReference layer,
}) async {
  // Read once, before the dialogue is built: a modal that asked the engine on
  // every rebuild would be a bridge call per keystroke.
  final name = layer.getName();
  SolidReference? solid;
  BridgeSolidDef? definition;
  try {
    if (layer.getSourceItem() case ItemReference_Solid(:final field0)) {
      solid = field0;
      definition = field0.getDefinition();
    }
  } catch (_) {
    // A layer whose source has gone is still a layer with a name.
  }

  final applied = await showLumitModal<bool>(
    context: context,
    id: 'layer-settings',
    builder: (close) => _LayerSettingsBody(
      name: name,
      definition: definition,
      onConfirm: (wanted, def) {
        try {
          if (wanted.isNotEmpty && wanted != name) layer.rename(name: wanted);
          if (solid != null && def != null && def != definition) {
            solid.setDefinition(definition: def);
          }
        } catch (_) {
          // The layer went away while the dialogue was open. Nothing to say
          // that the shell will not already be saying.
        }
        close(true);
      },
      onCancel: () => close(false),
    ),
  );
  return applied ?? false;
}

class _LayerSettingsBody extends StatefulWidget {
  final String name;
  final BridgeSolidDef? definition;
  final void Function(String, BridgeSolidDef?) onConfirm;
  final VoidCallback onCancel;

  const _LayerSettingsBody({
    required this.name,
    required this.definition,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_LayerSettingsBody> createState() => _LayerSettingsBodyState();
}

class _LayerSettingsBodyState extends State<_LayerSettingsBody> {
  late final TextEditingController _name =
      TextEditingController(text: widget.name);

  late BridgeColourRgba? _colour = widget.definition?.colour;
  late int _width = widget.definition?.width ?? 0;
  late int _height = widget.definition?.height ?? 0;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _confirm() {
    final was = widget.definition;
    widget.onConfirm(
      _name.text.trim(),
      was == null || _colour == null
          ? null
          : BridgeSolidDef(
              // The solid item keeps the layer's name: one thing, named once.
              name: _name.text.trim().isEmpty ? was.name : _name.text.trim(),
              colour: _colour!,
              width: _width < 1 ? 1 : _width,
              height: _height < 1 ? 1 : _height,
            ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final solid = widget.definition != null;
    return Focus(
      autofocus: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        if (event.logicalKey != LogicalKeyboardKey.enter &&
            event.logicalKey != LogicalKeyboardKey.numpadEnter) {
          return KeyEventResult.ignored;
        }
        _confirm();
        return KeyEventResult.handled;
      },
      child: FloatSurface(
        width: 360,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.only(bottom: 10),
              child: Text(l10n.menuLayerSettings, style: t.bodyPrimary),
            ),
            _row(
              t,
              label: l10n.name,
              child: HouseTextField(
                key: const ValueKey('layer-settings-name'),
                controller: _name,
                width: double.infinity,
                autofocus: true,
                onSubmitted: (_) => _confirm(),
              ),
            ),
            if (solid) ...[
              const SizedBox(height: 8),
              _row(
                t,
                label: l10n.sourceSolidSize,
                child: Row(
                  children: [
                    Expanded(
                      child: DragValueField(
                        key: const ValueKey('layer-settings-width'),
                        value: _width,
                        min: 1,
                        max: 16384,
                        decimals: 0,
                        onChanged: (v) => setState(() => _width = v.round()),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: DragValueField(
                        key: const ValueKey('layer-settings-height'),
                        value: _height,
                        min: 1,
                        max: 16384,
                        decimals: 0,
                        onChanged: (v) => setState(() => _height = v.round()),
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 8),
              _row(
                t,
                label: l10n.sourceSolidColour,
                child: Align(
                  alignment: Alignment.centerLeft,
                  // A solid's colour is chosen as a **display** colour, so the
                  // picker's channels read 0—255 — the same reading the
                  // source card's own swatch opens with.
                  child: ColourSwatchButton(
                    key: const ValueKey('layer-settings-colour'),
                    colour: PickedColour(
                            _colour?.r ?? 0, _colour?.g ?? 0, _colour?.b ?? 0)
                        .clipped,
                    onPicked: (picked) => setState(() => _colour =
                        BridgeColourRgba(
                            r: picked.r,
                            g: picked.g,
                            b: picked.b,
                            a: _colour?.a ?? 1)),
                  ),
                ),
              ),
            ],
            const SizedBox(height: 16),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('layer-settings-confirm'),
                  primary: true,
                  onPressed: _confirm,
                  child: Text(l10n.apply),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('layer-settings-cancel'),
                  onPressed: widget.onCancel,
                  child: Text(l10n.cancel),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  /// Label left, control right — the dialogue pattern's row (K-444).
  Widget _row(LumitTheme t, {required String label, required Widget child}) =>
      Row(
        children: [
          SizedBox(width: 110, child: Text(label, style: t.small)),
          Expanded(child: child),
        ],
      );
}
