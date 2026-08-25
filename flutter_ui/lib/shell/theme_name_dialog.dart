// Ask what a theme is called (K-202, K-298).
//
// One small dialogue with three callers — the editor's first Save, Rename, and
// Save a copy — because a theme's name is its identity: the picker shows it,
// the workspace file stores the selection by it, and two themes cannot share
// one. Three near-identical dialogues would have drifted apart in wording
// within a release.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

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
    builder: (close) => FloatSurface(
      width: 340,
      child: Padding(
        padding: const EdgeInsets.all(14),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(title, style: ThemeScope.of(context).theme.body),
            const SizedBox(height: 10),
            HouseTextField(
              key: const ValueKey('theme-name-field'),
              controller: controller,
              width: 300,
              autofocus: true,
              onSubmitted: close,
            ),
            const SizedBox(height: 12),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  small: true,
                  frameless: true,
                  onPressed: () => close(null),
                  child: Text(l10n.cancel),
                ),
                const SizedBox(width: 6),
                HouseButton(
                  key: const ValueKey('theme-name-ok'),
                  small: true,
                  // The default action (K-319). The name field holds focus, so
                  // Enter lands there and submits — the edge just says what
                  // Enter will do.
                  primary: true,
                  onPressed: () => close(controller.text),
                  child: Text(confirm ?? l10n.save),
                ),
              ],
            ),
          ],
        ),
      ),
    ),
  );
  controller.dispose();
  final trimmed = name?.trim();
  return (trimmed == null || trimmed.isEmpty) ? null : trimmed;
}
