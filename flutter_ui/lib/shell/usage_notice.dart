import 'dart:convert';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/usage_data.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

Future<void> maybeShowUsageNotice(
    BuildContext context, Workspace workspace) async {
  if (kIsWeb || workspace.usageNoticeSeen || !workspace.firstRunDone) return;
  await showLumitModal<void>(
    context: context,
    id: 'usage-notice',
    builder: (close) => _UsageNotice(onClose: () => close(null)),
  );
  workspace.setShareUsage(workspace.shareUsage);
}

class _UsageNotice extends StatelessWidget {
  final VoidCallback onClose;
  const _UsageNotice({required this.onClose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: 380,
      children: [
        dialogTitleBar(t,
            title: l10n.usageSentTitle,
            onClose: onClose,
            keyPrefix: 'usage-notice'),
        Padding(
          padding: const EdgeInsets.all(dialogPadding),
          child: Text(l10n.usageNotice, style: t.body),
        ),
        dialogFooter(
          t,
          keyPrefix: 'usage-notice',
          actions: [
            HouseButton(
              key: const ValueKey('usage-notice-ok'),
              primary: true,
              autofocus: true,
              onPressed: onClose,
              child: Text(l10n.usageOk),
            ),
          ],
        ),
      ],
    );
  }
}

Future<void> showUsageSent(BuildContext context, UsageData usage) {
  final text = const JsonEncoder.withIndent('  ').convert(usage.payload());
  return showLumitModal<void>(
    context: context,
    builder: (close) => _UsageSent(text: text, onClose: () => close(null)),
  );
}

class _UsageSent extends StatelessWidget {
  final String text;
  final VoidCallback onClose;
  const _UsageSent({required this.text, required this.onClose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: 420,
      children: [
        dialogTitleBar(t,
            title: l10n.usageSentTitle, onClose: onClose, keyPrefix: 'usage'),
        Padding(
          padding: const EdgeInsets.all(dialogPadding),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              ConstrainedBox(
                constraints: BoxConstraints(
                  maxHeight:
                      (MediaQuery.sizeOf(context).height * 0.6).clamp(120, 420),
                ),
                child: SingleChildScrollView(
                  child: SizedBox(
                    width: double.infinity,
                    child: Text(text, style: dialogMono(t)),
                  ),
                ),
              ),
              const SizedBox(height: 10),
              Text(l10n.usageSentNever, style: t.body),
            ],
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'usage',
          leading: HouseButton(
            key: const ValueKey('usage-copy'),
            onPressed: () => Clipboard.setData(ClipboardData(text: text)),
            child: Text(l10n.menuCopy),
          ),
          actions: [
            HouseButton(
              key: const ValueKey('usage-done'),
              autofocus: true,
              onPressed: onClose,
              child: Text(l10n.close),
            ),
          ],
        ),
      ],
    );
  }
}
