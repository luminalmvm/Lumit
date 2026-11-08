// The Grade panel's look picker: the presets in the library's Looks group, with
// Save look at the foot.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'grade_clipboard.dart' show looksGroup;

class LookPicker extends StatefulWidget {
  /// The look the layer is wearing, by name, if the panel knows of one.
  final String? current;
  final bool enabled;

  /// Null while the layer has no grade to save.
  final VoidCallback? onSave;
  final ValueChanged<BridgePresetInfo> onPick;

  const LookPicker({
    super.key,
    required this.current,
    required this.enabled,
    required this.onSave,
    required this.onPick,
  });

  @override
  State<LookPicker> createState() => _LookPickerState();
}

class _LookPickerState extends State<LookPicker> {
  List<BridgePresetInfo> _looks = const [];
  LumitUiState? _ui;

  @override
  void initState() {
    super.initState();
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _ui!.presetsChanged.addListener(_read);
    _read();
  }

  @override
  void dispose() {
    _ui?.presetsChanged.removeListener(_read);
    super.dispose();
  }

  void _read() {
    final looks = [
      for (final p in listPresets())
        if (p.group == looksGroup || p.group.startsWith('$looksGroup/')) p,
    ];
    if (mounted) setState(() => _looks = looks);
  }

  Future<void> _open() async {
    final box = context.findRenderObject()! as RenderBox;
    await showLumitPopup<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final look in _looks)
                MenuRow(
                  key: ValueKey<String>('grade-look-${look.name}'),
                  onPressed: () {
                    close(null);
                    widget.onPick(look);
                  },
                  child:
                      MenuLabel(look.name, ticked: look.name == widget.current),
                ),
              if (_looks.isNotEmpty) const MenuDivider(),
              MenuRow(
                key: const ValueKey('grade-look-save'),
                enabled: widget.onSave != null,
                onPressed: () {
                  close(null);
                  widget.onSave?.call();
                },
                child: MenuLabel(l10n.gradeSaveLook,
                    enabled: widget.onSave != null,
                    ticked: _looks.isEmpty ? null : false),
              ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return dropdownButton(
      t: t,
      dense: false,
      onPressed: widget.enabled && (_looks.isNotEmpty || widget.onSave != null)
          ? _open
          : null,
      face: dropdownFace(t, widget.current ?? l10n.none),
    );
  }
}
