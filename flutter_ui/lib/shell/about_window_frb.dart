// About Lumit — the window Help ▸ About Lumit opens.
//
// It used to be a section at the bottom of Settings ▸ General, which is where
// nobody looks for it: Settings is for things you change, and none of this is.
// The boot log is here rather than in the Debug panel because the first line of
// it *is* the version, and the rest is what someone pastes into a bug report.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'package:lumit_flutter/src/rust/api/shell.dart';

import '../l10n/strings.dart';
import '../state/updates.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'wordmark.dart';

Future<void> showAboutWindowFrb(BuildContext context) => showLumitModal<void>(
      context: context,
      builder: (close) => _AboutWindow(onClose: () => close(null)),
    );

/// This build of Lumit, as the engine reports it on boot. "Unknown" rather than
/// an empty line when the log is empty, which is what a test harness sees.
///
/// It is the *boot line*, so it names the library that printed it —
/// `lumit-bridge 0.2.0`. That is the right thing in a bug report and the wrong
/// thing anywhere a person is being told which Lumit they have; use
/// [lumitProductVersion] for that.
String lumitVersion() => bootLog().isEmpty ? 'unknown' : bootLog().first;

/// **The product's version**, as somebody would say it out loud: `Lumit 0.2.0`.
///
/// The number is the boot line's, and that is not a shortcut: the whole
/// repository is versioned together — `Cargo.toml`'s `workspace.package` and
/// `flutter_ui/pubspec.yaml` carry the same three digits, and the release tag
/// the updater compares against is the same number again. So there is one
/// version, and only the crate's name was leaking out of the boot line.
///
/// Written here rather than in each caller: Settings ▸ General and the welcome
/// screen both say it, and two spellings of the same fact is one bug report
/// nobody can read.
String lumitProductVersion() =>
    'Lumit ${versionFromBootLine(lumitVersion()) ?? '?'}';

class _AboutWindow extends StatelessWidget {
  final VoidCallback onClose;
  const _AboutWindow({required this.onClose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final log = bootLog();
    return DialogFrame(
      width: 420,
      children: [
        dialogTitleBar(t,
            title: l10n.menuAboutLumit, onClose: onClose, keyPrefix: 'about'),
        Padding(
          padding: const EdgeInsets.all(dialogPadding),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  LumitWordmark(height: 22, ground: t.surface1),
                  const SizedBox(width: 8),
                  Text(versionFromBootLine(lumitVersion()) ?? '',
                      style: dialogMono(t)),
                ],
              ),
              const SizedBox(height: 10),
              Text(l10n.aboutBlurb, style: t.body),
              if (log.length > 1) const SizedBox(height: 10),
              for (final line in log.skip(1))
                Padding(
                  padding: const EdgeInsets.only(bottom: 2),
                  child: Text(line, style: dialogMono(t)),
                ),
            ],
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'about',
          // The boot log is what goes in a bug report.
          leading: log.isEmpty
              ? null
              : HouseButton(
                  key: const ValueKey('about-copy'),
                  onPressed: () =>
                      Clipboard.setData(ClipboardData(text: log.join('\n'))),
                  child: Text(l10n.menuCopy),
                ),
          actions: [
            HouseButton(
              key: const ValueKey('about-done'),
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
