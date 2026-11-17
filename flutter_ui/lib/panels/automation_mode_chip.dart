// A track's automation mode on its head, in the Audio timeline: the mode's
// name in caption type, coloured by what the mode does, and a click that
// picks another (docs/impl/audio-timeline.md §5).

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_routes_frb.dart' show automationModeLabel;
import 'timeline_extras_frb.dart' show showMenuAt;

/// The cell the chip stands in, beside the switches.
const double automationChipWidth = 34;

/// What [mode] is drawn in: Write in the error tone, the modes that write
/// once touched in the warning tone, and Read quiet unless the track's
/// values are keyed, which is when Read is doing something.
Color automationModeInk(LumitTheme t, BridgeAutomationMode mode,
        {required bool keyed}) =>
    switch (mode) {
      BridgeAutomationMode.write => t.error,
      BridgeAutomationMode.touch ||
      BridgeAutomationMode.latch ||
      BridgeAutomationMode.trim =>
        t.warning,
      BridgeAutomationMode.read => keyed ? t.accent : t.textMuted,
    };

/// The five modes as menu rows, [current] ticked.
List<Widget> automationModeRows({
  required String keyPrefix,
  required BridgeAutomationMode current,
  required VoidCallback dismiss,
  required ValueChanged<BridgeAutomationMode> onPick,
}) =>
    [
      for (final mode in BridgeAutomationMode.values)
        MenuRow(
          key: ValueKey<String>('$keyPrefix-${mode.name}'),
          onPressed: () {
            dismiss();
            if (mode != current) onPick(mode);
          },
          child: MenuLabel(automationModeLabel(mode), ticked: mode == current),
        ),
    ];

/// The chip itself. [onPick] writes the mode chosen from its menu.
class AutomationModeChip extends StatelessWidget {
  final String id;
  final BridgeAutomationMode mode;
  final bool keyed;
  final ValueChanged<BridgeAutomationMode> onPick;

  const AutomationModeChip({
    super.key,
    required this.id,
    required this.mode,
    required this.keyed,
    required this.onPick,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ink = automationModeInk(t, mode, keyed: keyed);
    final quiet = mode == BridgeAutomationMode.read && !keyed;
    return LumitTooltip(
      message: l10n.audioAutomation,
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          key: ValueKey<String>('atl-automation-$id'),
          behavior: HitTestBehavior.opaque,
          onTapUp: (d) => showMenuAt<void>(
            context: context,
            position: d.globalPosition,
            width: 130,
            rows: (close) => automationModeRows(
              keyPrefix: 'atl-automation-$id',
              current: mode,
              dismiss: () => close(null),
              onPick: onPick,
            ),
          ),
          child: SizedBox(
            width: automationChipWidth,
            height: t.density.laneRow,
            child: Center(
              child: Text(
                automationModeLabel(mode),
                style: t.caption.copyWith(
                    color: quiet ? Color.lerp(ink, t.textPrimary, lit) : ink),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
