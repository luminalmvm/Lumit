// Ask what a theme is called.
//
// One small dialogue with three callers — the editor's first Save, Rename, and
// Save a copy — because a theme's name is its identity: the picker shows it,
// the workspace file stores the selection by it, and two themes cannot share
// one. Three near-identical dialogues would have drifted apart in wording
// within a release.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// Ask for a theme name, seeded with [suggested] and headed by [title].
///
/// Returns the trimmed name, or null when the dialogue was dismissed or left
/// blank — a theme with no name could not be selected again, so a blank is the
/// same answer as cancelling rather than a theme called nothing.
Future<String?> askThemeName(
  BuildContext context, {
  required String title,
  required String suggested,
  String? confirm,
}) async {
  final controller = TextEditingController(text: suggested);
  final name = await showLumitModal<String>(
    context: context,
    builder: (close) => Builder(builder: (context) {
      final t = ThemeScope.of(context).theme;
      return DialogFrame(
        width: 340,
        children: [
          dialogTitleBar(t,
              title: title,
              onClose: () => close(null),
              keyPrefix: 'theme-name'),
          Padding(
            padding: dialogBodyPadding,
            child: HouseTextField(
              key: const ValueKey('theme-name-field'),
              controller: controller,
              width: double.infinity,
              autofocus: true,
              onSubmitted: close,
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'theme-name',
            actions: [
              HouseButton(
                key: const ValueKey('theme-name-cancel'),
                onPressed: () => close(null),
                child: Text(l10n.cancel),
              ),
              HouseButton(
                key: const ValueKey('theme-name-ok'),
                // The default action. The name field holds focus, so Enter
                // lands there and submits: the fill just says what Enter does.
                primary: true,
                onPressed: () => close(controller.text),
                child: Text(confirm ?? l10n.save),
              ),
            ],
          ),
        ],
      );
    }),
  );
  controller.dispose();
  final trimmed = name?.trim();
  return (trimmed == null || trimmed.isEmpty) ? null : trimmed;
}
