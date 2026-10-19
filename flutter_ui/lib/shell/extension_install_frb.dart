// Installing an extension from a folder or a `.lfx` package: read what it
// asks to be allowed, and agree or not. Nothing is copied before the person
// agrees. Making a package lives here too.

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
      BridgeExtensionPermission.edit => l10n.extensionCanEdit,
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
  final read = await service.inspect(folder);
  if (!context.mounted) return;
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
  _say(app, await service.install(folder));
}

/// Pick a `.lfx` package and install it, as [installExtensionPackage] does.
Future<void> installExtensionFromPackage(
    BuildContext context, LumitState app, ExtensionService service) async {
  final path = await pickExtensionPackage();
  if (path == null || !context.mounted) return;
  await installExtensionPackage(context, app, service, path);
}

/// Install the package at [path] once the person has read what it adds and
/// agreed, into Lumit's own folder or one they pick. A double-clicked or
/// dropped package comes here too.
Future<void> installExtensionPackage(BuildContext context, LumitState app,
    ExtensionService service, String path) async {
  final read = await service.inspectPackage(path);
  if (!context.mounted) return;
  if (read is! BridgeExtensionOutcome_Ready) {
    return app.postNotice(extensionOutcomeText(read), error: true);
  }
  final own = service.scanFolders.own;
  final into = ValueNotifier<String?>(own);
  final agreed = await showLumitModal<bool>(
    context: context,
    id: 'extension-install',
    builder: (close) => _InstallQuestion(
        extension: read.extension_, into: into, onChoose: close),
  );
  final folder = into.value;
  into.dispose();
  if (agreed != true) return;
  _say(
      app,
      await service.installPackage(path,
          folder: folder == own ? null : folder));
}

void _say(LumitState app, BridgeExtensionOutcome outcome) {
  if (outcome case BridgeExtensionOutcome_Ready(extension_: final extension)) {
    app.postNotice(extension.panel
        ? l10n.extensionInstalled(extension.name)
        : l10n.extensionInstalledPlain(extension.name));
  } else {
    app.postNotice(extensionOutcomeText(outcome), error: true);
  }
}

/// Pick an extension's folder and save it as `<id>.lfx` where the person
/// says, checked as an install would check it.
Future<void> makeExtensionPackage(
    LumitState app, ExtensionService service) async {
  final folder = await pickFolder();
  if (folder == null) return;
  final read = await service.inspect(folder);
  if (read is! BridgeExtensionOutcome_Ready) {
    return app.postNotice(extensionOutcomeText(read), error: true);
  }
  final out =
      await pickExtensionPackageSaveLocation('${read.extension_.id}.lfx');
  if (out == null) return;
  final outcome = await service.makePackage(folder, out);
  if (outcome case BridgeExtensionOutcome_Ready(extension_: final extension)) {
    app.postNotice(l10n.extensionPackageMade(extension.name));
  } else {
    app.postNotice(extensionOutcomeText(outcome), error: true);
  }
}

class _InstallQuestion extends StatelessWidget {
  final BridgeExtension extension;
  final ValueChanged<bool?> onChoose;

  /// Where a package goes, which the person may change. Null for a folder
  /// install, which always goes into Lumit's own folder.
  final ValueNotifier<String?>? into;

  const _InstallQuestion(
      {required this.extension, required this.onChoose, this.into});

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
                if (extension.effects.isNotEmpty)
                  point(l10n.extensionCanEffects(
                      extension.effects.map((e) => e.name).join(', '))),
                for (final permission in extension.permissions)
                  point(extensionPermissionText(permission)),
                if (extension.hosts.isNotEmpty)
                  point(l10n.extensionCanHosts(extension.hosts.join(', '))),
                if (extension.panel) point(l10n.extensionCanPanel),
              ]),
              dialogGroup(t, l10n.extensionCannot, [
                point(l10n.extensionCannotFootage),
                point(l10n.extensionCannotFiles),
                if (extension.permissions
                    .contains(BridgeExtensionPermission.edit))
                  point(l10n.extensionCannotSave)
                // An effect's own interface sets that effect's rows.
                else if (!extension.effects.any((e) => e.interface_))
                  point(l10n.extensionCannotEdit),
              ]),
              if (into case final into?)
                ValueListenableBuilder<String?>(
                  valueListenable: into,
                  builder: (context, folder, _) =>
                      dialogGroup(t, l10n.extensionInstallsTo, [
                    Row(
                      children: [
                        Expanded(
                          child: Text(folder ?? '',
                              key: const ValueKey('extension-install-folder'),
                              style: t.small),
                        ),
                        const SizedBox(width: 8),
                        HouseButton(
                          key: const ValueKey('extension-install-change'),
                          small: true,
                          onPressed: () async {
                            final picked = await pickFolder();
                            if (picked != null) into.value = picked;
                          },
                          child: Text(l10n.changeEllipsis, style: t.small),
                        ),
                      ],
                    ),
                  ]),
                ),
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
