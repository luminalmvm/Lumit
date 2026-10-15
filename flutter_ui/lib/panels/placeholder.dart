// The shared placeholder a panel shows before its phase makes it live
// (docs/archive/flutter-port/05-PARITY-CHECKLIST.md).

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../widgets/controls.dart';

class PlaceholderPanel extends StatelessWidget {
  final LumitIcon icon;
  final String title;
  final String hint;

  /// A button under the hint, when there is something to do about it.
  final Widget? action;

  const PlaceholderPanel({
    super.key,
    required this.icon,
    required this.title,
    required this.hint,
    this.action,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          lumitIcon(icon, size: 28, color: t.textDisabled),
          const SizedBox(height: 8),
          Text(title, style: t.body),
          const SizedBox(height: 4),
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 220),
            child: Text(hint, style: t.small, textAlign: TextAlign.center),
          ),
          if (action case final action?) ...[
            const SizedBox(height: 8),
            action,
          ],
        ],
      ),
    );
  }
}
