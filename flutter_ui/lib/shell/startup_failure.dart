// The screen Lumit shows when it could not start.
//
// # In plain terms
//
// The Windows window stays invisible until Flutter draws its first frame, and
// nothing draws a frame until `runApp` is reached. So anything that goes wrong
// on the way up — the engine library refusing to load, a settings read that
// throws, a bridge call that fails — used to leave Lumit as a line in Task
// Manager and nothing at all on screen, with no message anywhere a person
// could find it. That is the bug this file answers (reported against 0.3.0).
//
// It does one thing and no more: put the failure on screen in a window that can
// be read, copied and closed. Writing it down is `state/faults.dart`'s job
// already, and `main` calls that recorder on the way here, so both halves of a
// bad start end up in the same file.

import 'package:flutter/material.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/state/faults.dart';
import 'package:lumit_flutter/theme/theme.dart';

/// The whole application, when there is no application: a heading, a line of
/// explanation, and the error itself where it can be selected and copied.
class StartupFailureApp extends StatelessWidget {
  const StartupFailureApp(this.detail, {super.key});

  /// What went wrong, as text. The stack trace stays in the file — a person
  /// looking at this needs the sentence, not the frames.
  final String detail;

  @override
  Widget build(BuildContext context) {
    // The dark scheme rather than the user's: their settings are among the
    // things that may not have loaded.
    final theme = LumitTheme.dark();
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      localizationsDelegates: Strings.localizationsDelegates,
      supportedLocales: Strings.supportedLocales,
      home: ColoredBox(
        color: theme.surface0,
        child: Padding(
          padding: const EdgeInsets.all(32),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.startupFailedTitle,
                style: TextStyle(color: theme.textPrimary, fontSize: 20),
              ),
              const SizedBox(height: 12),
              Text(
                l10n.startupFailedBody,
                style: TextStyle(color: theme.textSecondary, fontSize: 13),
              ),
              const SizedBox(height: 20),
              Expanded(
                child: SingleChildScrollView(
                  child: SelectableText(
                    '$detail\n\n${faultLog().path}',
                    style: TextStyle(color: theme.textPrimary, fontSize: 12),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
