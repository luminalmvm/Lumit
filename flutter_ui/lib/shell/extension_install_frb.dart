// Installing an extension from a folder: pick it, read what it asks to be
// allowed, and agree or not. Nothing is copied before the person agrees.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';

import '../l10n/strings.dart';
import '../state/extensions.dart';
import '../state/file_dialogs.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// What a permission lets an extension do, as a line the person reads.
String extensionPermissionText(BridgeExtensionPermission permission) =>
    switch (permission) {
      BridgeExtensionPermission.project => l10n.extensionCanProject,
      BridgeExtensionPermission.activity => l10n.extensionCanActivity,
      BridgeExtensionPermission.export_ => l10n.extensionCanExport,
      BridgeExtensionPermission.import_ => l10n.extensionCanImport,
      BridgeExtensionPermission.share => l10n.extensionCanShare,
    };

/// Why an extension could not be read or installed, as a sentence.
String extensionOutcomeText(BridgeExtensionOutcome outcome) =>
    switch (outcome) {
      BridgeExtensionOutcome_Ready() => '',
      BridgeExtensionOutcome_NotAnExtension() => l10n.extensionNotOne,
      // The engine's own sentence, which names the rule the manifest broke.
      BridgeExtensionOutcome_Invalid(:final why) => why,
      BridgeExtensionOutcome_Newer(:final version) =>
        l10n.extensionNewer(version),
      BridgeExtensionOutcome_TooLarge() => l10n.extensionTooLarge,
      BridgeExtensionOutcome_Busy() => l10n.extensionBusy,
      BridgeExtensionOutcome_Failed() => l10n.extensionFailed,
    };

/// Pick a folder and install the extension in it, once the person has read
/// what it asks for and agreed. Says how it went in the bottom strip.
Future<void> installExtensionFromFolder(
    BuildContext context, LumitState app, ExtensionService service) async {
  final folder = await pickFolder();
  if (folder == null || !context.mounted) return;
  final read = service.inspect(folder);
  if (read is! BridgeExtensionOutcome_Ready) {
    return app.postNotice(extensionOutcomeText(read), error: true);
  }
  final agreed = await showLumitModal<bool>(
    context: context,
    id: 'extension-install',
    builder: (close) =>
        _InstallQuestion(extension: read.extension_, onChoose: close),
  );
  if (agreed != true) return;
  final outcome = await service.install(folder);
  if (outcome case BridgeExtensionOutcome_Ready(extension_: final extension)) {
    app.postNotice(l10n.extensionInstalled(extension.name));
  } else {
    app.postNotice(extensionOutcomeText(outcome), error: true);
  }
}

class _InstallQuestion extends StatelessWidget {
  final BridgeExtension extension;
  final ValueChanged<bool?> onChoose;

  const _InstallQuestion({required this.extension, required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget point(String text) => Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('·  ', style: t.small.copyWith(color: t.textMuted)),
              Expanded(child: Text(text, style: t.small)),
            ],
          ),
        );
    final muted = t.small.copyWith(color: t.textMuted);
    return DialogFrame(
      width: 440,
      children: [
        dialogTitleBar(
          t,
          title: l10n.extensionInstallTitle(extension.name),
          onClose: () => onChoose(false),
          keyPrefix: 'extension-install',
        ),
        Padding(
          padding: const EdgeInsets.all(dialogPadding),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                extension.author.isEmpty
                    ? l10n.extensionVersion(extension.version)
                    : l10n.extensionVersionBy(
                        extension.version, extension.author),
                style: muted,
              ),
              if (extension.summary.isNotEmpty) ...[
                const SizedBox(height: 6),
                Text(extension.summary, style: t.body),
              ],
              dialogGroup(t, l10n.extensionCan, [
                for (final permission in extension.permissions)
                  point(extensionPermissionText(permission)),
                if (extension.hosts.isNotEmpty)
                  point(l10n.extensionCanHosts(extension.hosts.join(', '))),
                point(l10n.extensionCanPanel),
              ]),
              dialogGroup(t, l10n.extensionCannot, [
                point(l10n.extensionCannotFootage),
                point(l10n.extensionCannotFiles),
                point(l10n.extensionCannotEdit),
              ]),
              const SizedBox(height: 10),
              Text(l10n.extensionWebNote, style: muted),
            ],
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'extension-install',
          actions: [
            HouseButton(
              key: const ValueKey('extension-install-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => onChoose(false),
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('extension-install-confirm'),
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: () => onChoose(true),
              child: Text(l10n.extensionInstall),
            ),
          ],
        ),
      ],
    );
  }
}
