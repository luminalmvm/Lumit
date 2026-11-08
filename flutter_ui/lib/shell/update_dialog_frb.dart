// The windows an update puts up, and the order they come in.
//
// # In plain terms
//
// `state/updates.dart` knows how to find a newer Lumit and fetch it. This file
// is the part the user sees: the one question before a few hundred megabytes
// are downloaded, the progress while it comes, and the restart at the end —
// with the offer to save first, because the update cannot finish while Lumit is
// running and nobody should lose an evening's work to a version number. A
// release with something to say before it is applied says it ahead of the
// restart question, on a window of its own.
//
// Nothing here decides anything about updating. It asks, it shows, and it calls
// back into the service, so the two can be read separately: what an update *is*
// over there, what it *looks like* here.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/updates.dart';
import '../widgets/controls.dart';
import 'question_window.dart';

/// Act on the Help ▸ Check for updates row, whatever it currently says.
///
/// One entry point for the menu row and the Settings button both, so the two
/// cannot come to mean different things. [saveProject] is passed in rather than
/// reached for: saving belongs to the shell's File menu, and a dialogue that
/// imported it would tie this file to the menu bar that calls it.
Future<void> pressUpdateRow(
  BuildContext context, {
  required UpdateService updates,
  required void Function(String message, {bool error}) notice,
  required bool Function() projectIsDirty,
  required Future<void> Function() saveProject,
}) async {
  // In flight: the row is disabled anyway, and a second press should not start
  // a second check.
  if (updates.busy) return;

  if (updates.stage == UpdateStage.available) {
    await _offerAndFetch(
      context,
      updates: updates,
      notice: notice,
      projectIsDirty: projectIsDirty,
      saveProject: saveProject,
    );
    return;
  }

  if (updates.stage == UpdateStage.ready) {
    await _askAboutRestart(
      context,
      updates: updates,
      projectIsDirty: projectIsDirty,
      saveProject: saveProject,
    );
    return;
  }

  // Idle, up to date, or a check that did not finish: all three are "ask
  // again", and what comes back is said in the status line as well as in the
  // row, because the row is a menu somebody has probably just closed.
  await updates.check();
  switch (updates.stage) {
    case UpdateStage.upToDate:
      notice(l10n.updateNoneAvailable);
    case UpdateStage.failed:
      notice(updates.failure ?? l10n.updateCheckFailed, error: true);
    case UpdateStage.available:
      notice(l10n.updateIsAvailable('${updates.release?.version}'));
    default:
      break;
  }
}

/// Ask before updates follow nightly builds. True when the user agreed.
///
/// The one thing worth a window: a nightly can save a project the stable
/// version cannot open, and that is better heard before the switch than after.
Future<bool> confirmNightlyBuilds(BuildContext context) async {
  final go = await showLumitModal<bool>(
    context: context,
    builder: (close) => _Ask(
      title: l10n.updateNightlyTitle,
      body: l10n.updateNightlyBody,
      yes: l10n.updateContinue,
      onChoose: close,
    ),
  );
  return go == true;
}

/// Act on the bottom strip's update button.
///
/// The same road as the row without the two windows in the middle. Pressing
/// a download arrow is already the yes, and the button draws the progress
/// itself, so the work carries on underneath. The restart is still asked
/// about, on the next press.
Future<void> pressUpdateButton(
  BuildContext context, {
  required UpdateService updates,
  required void Function(String message, {bool error}) notice,
  required bool Function() projectIsDirty,
  required Future<void> Function() saveProject,
}) async {
  if (updates.stage != UpdateStage.available) {
    return pressUpdateRow(
      context,
      updates: updates,
      notice: notice,
      projectIsDirty: projectIsDirty,
      saveProject: saveProject,
    );
  }
  await updates.downloadUpdate();
  if (updates.stage == UpdateStage.failed) {
    notice(updates.failure ?? l10n.updateDownloadFailed, error: true);
  } else if (updates.stage == UpdateStage.ready) {
    notice(l10n.updateDownloaded('${updates.release?.version}'));
  }
}

/// Ask, download, then move on to the restart question.
Future<void> _offerAndFetch(
  BuildContext context, {
  required UpdateService updates,
  required void Function(String message, {bool error}) notice,
  required bool Function() projectIsDirty,
  required Future<void> Function() saveProject,
}) async {
  final release = updates.release;
  if (release == null) return;

  final go = await showLumitModal<bool>(
    context: context,
    builder: (close) => _Ask(
      title: l10n.updateOfferTitle(release.version),
      summary: release.sizeLabel,
      yes: l10n.updateDownload,
      onChoose: close,
    ),
  );
  if (go != true || !context.mounted) return;

  // Started first, shown second: the dialogue watches the service, and the
  // service is what is doing the work.
  final downloading = updates.downloadUpdate();
  // Dismissing this window (the scrim, or Cancel arriving late) does not stop
  // the download — the service owns that, and only the Cancel button asks it
  // to stop. So the flow waits for the download itself below, whichever way the
  // window went.
  await showLumitModal<void>(
    context: context,
    builder: (close) => _DownloadProgress(
      updates: updates,
      release: release,
      onDone: () => close(null),
      onCancel: updates.cancelDownload,
    ),
  );
  await downloading;

  if (updates.stage == UpdateStage.failed) {
    notice(updates.failure ?? l10n.updateDownloadFailed, error: true);
    return;
  }
  if (updates.stage != UpdateStage.ready || !context.mounted) return;
  await _askAboutRestart(
    context,
    updates: updates,
    projectIsDirty: projectIsDirty,
    saveProject: saveProject,
  );
}

/// The last window: restart now, or later, and save first if there is work
/// open.
Future<void> _askAboutRestart(
  BuildContext context, {
  required UpdateService updates,
  required bool Function() projectIsDirty,
  required Future<void> Function() saveProject,
}) async {
  // What the release says to read first, on its own window and ahead of the
  // restart question, every time the row is pressed: someone who chose Later
  // yesterday is being asked the same thing today.
  final notice = updates.notice;
  if (notice.isNotEmpty) {
    final read = await showLumitModal<bool>(
      context: context,
      builder: (close) => _BeforeYouUpdate(
        version: updates.release?.version ?? '',
        lines: notice,
        onChoose: close,
      ),
    );
    // Not now, Escape and the scrim all leave the update where it is,
    // downloaded and still offered by the row.
    if (read != true || !context.mounted) return;
  }

  final answer = await showLumitModal<_RestartAnswer>(
    context: context,
    builder: (close) => _RestartToFinish(
      version: updates.release?.version ?? '',
      dirty: projectIsDirty(),
      delivery: updates.delivery,
      onChoose: close,
    ),
  );
  // Later keeps the downloaded installer and the row that offers it: the
  // update is still ready, and the next press of it comes straight back here.
  if (answer == null || answer == _RestartAnswer.later) return;
  if (answer == _RestartAnswer.saveAndRestart) await saveProject();
  await updates.install();
}

/// What the restart window can be answered with.
enum _RestartAnswer { restart, saveAndRestart, later }

/// What the release says to read before it is applied. Only ever shown for a
/// release with something to say, and always ahead of the restart question.
class _BeforeYouUpdate extends StatelessWidget {
  final String version;
  final List<UpdateNoticeLine> lines;
  final ValueChanged<bool?> onChoose;

  const _BeforeYouUpdate({
    required this.version,
    required this.lines,
    required this.onChoose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return QuestionWindow(
      width: 460,
      title: l10n.updateNoticeTitle(version),
      keyPrefix: 'update-notice-window',
      onClose: () => onChoose(false),
      actions: [
        questionPlain(
            'update-notice-not-now', l10n.updateNotNow, () => onChoose(false)),
        questionFilled(
            'update-notice-continue', l10n.updateContinue, () => onChoose(true)),
      ],
      // Capped and scrolling, so a release with a lot to say does not push the
      // buttons off the screen.
      content: ConstrainedBox(
        key: const ValueKey('update-notice'),
        constraints: const BoxConstraints(maxHeight: 220),
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final (index, line) in lines.indexed)
                Padding(
                  padding: EdgeInsets.only(top: index == 0 ? 0 : 6),
                  child: line.bullet
                      ? Row(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            SizedBox(
                              width: 12,
                              child: Text('•', style: t.body),
                            ),
                            Expanded(child: Text(line.text, style: t.body)),
                          ],
                        )
                      : Text(line.text, style: t.body),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// A question with [summary] in its footer: "Not now", and the way forward.
/// The offer of an update and the nightly warning are both this.
class _Ask extends StatelessWidget {
  final String title;
  final String? body;
  final String summary;

  /// The label on the button that goes ahead.
  final String yes;
  final ValueChanged<bool?> onChoose;

  const _Ask({
    required this.title,
    this.body,
    this.summary = '',
    required this.yes,
    required this.onChoose,
  });

  @override
  Widget build(BuildContext context) => QuestionWindow(
        title: title,
        body: body,
        summary: summary,
        keyPrefix: 'update-offer',
        onClose: () => onChoose(false),
        actions: [
          questionPlain(
              'update-offer-no', l10n.updateNotNow, () => onChoose(false)),
          questionFilled('update-offer-yes', yes, () => onChoose(true)),
        ],
      );
}

/// The download, while it happens. Closes itself the moment the service leaves
/// the downloading stage, however it left: finished, cancelled or failed.
class _DownloadProgress extends StatelessWidget {
  final UpdateService updates;
  final UpdateRelease release;
  final VoidCallback onDone;
  final VoidCallback onCancel;

  const _DownloadProgress({
    required this.updates,
    required this.release,
    required this.onDone,
    required this.onCancel,
  });

  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: updates,
        builder: (context, _) {
          if (updates.stage != UpdateStage.downloading) {
            // Over before this window reached the screen, or over now. Either
            // way the close waits a frame, because a window must not dispose
            // itself mid-build. `close` is idempotent, so a repeat callback is
            // a no-op rather than a double dismissal.
            WidgetsBinding.instance.addPostFrameCallback((_) => onDone());
          }
          final fraction = updates.progress;
          final done = (updates.downloadBytes * fraction / (1 << 20)).round();
          return QuestionWindow(
            title: l10n.updateDownloading(release.version),
            summary: l10n.updateProgressOf(
                '$done', megabytes(updates.downloadBytes)),
            keyPrefix: 'update-download',
            onClose: onCancel,
            content: HouseProgressBar(fraction: fraction, height: 6),
            actions: [
              questionPlain('update-download-cancel', l10n.cancel, onCancel),
            ],
          );
        },
      );
}

/// The end of the update: it is on disk, and applying it means closing Lumit.
class _RestartToFinish extends StatelessWidget {
  final String version;

  /// Whether the open project has unsaved work, which is what puts the save
  /// button on this window rather than leaving the choice to be regretted.
  final bool dirty;

  /// How the update will be applied: a swap and a restart, an installer and a
  /// restart, or a file handed to Flatpak while Lumit stays open.
  final UpdateDelivery delivery;

  final ValueChanged<_RestartAnswer?> onChoose;

  const _RestartToFinish({
    required this.version,
    required this.dirty,
    required this.delivery,
    required this.onChoose,
  });

  /// Whether finishing means leaving. False only for the Flatpak hand-off.
  bool get quits => delivery != UpdateDelivery.flatpakBundle;

  @override
  Widget build(BuildContext context) => QuestionWindow(
        width: 460,
        // Three answers, and one of them a whole phrase.
        stacked: quits && dirty,
        title:
            quits ? l10n.updateRestartToFinish : l10n.updateDownloaded(version),
        // Inside the sandbox the files are not ours to replace, so the bundle
        // is handed over and Flatpak does the rest. Anywhere else the one thing
        // to know is that there is work to lose.
        body: !quits
            ? l10n.updateReadyFlatpak(version)
            : dirty
                ? l10n.updateUnsavedChanges
                : null,
        keyPrefix: 'update-restart',
        onClose: () => onChoose(_RestartAnswer.later),
        actions: [
          questionPlain('update-restart-later', l10n.updateLater,
              () => onChoose(_RestartAnswer.later)),
          if (quits && dirty) ...[
            questionPlain('update-restart-now', l10n.updateRestartWithoutSaving,
                () => onChoose(_RestartAnswer.restart)),
            // The default: Enter takes the safe restart, the one that saves.
            questionFilled('update-save-restart', l10n.updateSaveAndRestart,
                () => onChoose(_RestartAnswer.saveAndRestart)),
          ] else
            questionFilled(
                'update-restart-now',
                quits ? l10n.updateRestartNow : l10n.updateShowTheFile,
                () => onChoose(_RestartAnswer.restart)),
        ],
      );
}
