// The windows an update puts up, and the order they come in (K-296).
//
// # In plain terms
//
// `state/updates.dart` knows how to find a newer Lumit and fetch it. This file
// is the part the user sees: the one question before a few hundred megabytes
// are downloaded, the progress while it comes, and the restart at the end —
// with the offer to save first, because the update cannot finish while Lumit is
// running and nobody should lose an evening's work to a version number.
//
// Nothing here decides anything about updating. It asks, it shows, and it calls
// back into the service, so the two can be read separately: what an update *is*
// over there, what it *looks like* here.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/updates.dart';
import '../widgets/controls.dart';

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
    builder: (close) => _OfferUpdate(release: release, onChoose: close),
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

/// "There is a newer Lumit — shall I fetch it?", with what that costs.
class _OfferUpdate extends StatelessWidget {
  final UpdateRelease release;
  final ValueChanged<bool?> onChoose;

  const _OfferUpdate({required this.release, required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 400,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(l10n.updateOfferTitle(release.version),
                style: t.bodyPrimary),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(
              l10n.updateOfferBody(release.assetName, release.sizeLabel),
              style: t.small.copyWith(color: t.textMuted),
            ),
          ),
          const SizedBox(height: 14),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('update-offer-no'),
                  small: true,
                  frameless: true,
                  onPressed: () => onChoose(false),
                  child: Text(l10n.updateNotNow, style: t.small),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('update-offer-yes'),
                  small: true,
                  // The window's default action (K-319): Enter downloads.
                  primary: true,
                  autofocus: true,
                  onPressed: () => onChoose(true),
                  child: Text(l10n.updateDownload, style: t.small),
                ),
              ],
            ),
          ),
          const SizedBox(height: 10),
        ],
      ),
    );
  }
}

/// The download, while it happens. Closes itself the moment the service leaves
/// the downloading stage, however it left — finished, cancelled or failed.
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
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: updates,
      builder: (context, _) {
        if (updates.stage != UpdateStage.downloading) {
          // Over before this window reached the screen, or over now — either
          // way the close waits a frame, because a window must not dispose
          // itself mid-build. `close` is idempotent, so a repeat callback is
          // a no-op rather than a double dismissal.
          WidgetsBinding.instance.addPostFrameCallback((_) => onDone());
        }
        final fraction = updates.progress;
        final done = (release.assetBytes * fraction / (1 << 20)).round();
        return FloatSurface(
          width: 400,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.all(10),
                child: Text(l10n.updateDownloading(release.version),
                    style: t.bodyPrimary),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 10),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    HouseProgressBar(fraction: fraction, height: 6),
                    const SizedBox(height: 6),
                    Text(
                      l10n.updateProgressOf('$done', release.sizeLabel),
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 14),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 10),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    HouseButton(
                      key: const ValueKey('update-download-cancel'),
                      small: true,
                      frameless: true,
                      onPressed: onCancel,
                      child: Text(l10n.cancel, style: t.small),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 10),
            ],
          ),
        );
      },
    );
  }
}

/// The end of the update: it is on disk, and applying it means closing Lumit.
class _RestartToFinish extends StatelessWidget {
  final String version;

  /// Whether the open project has unsaved work, which is what puts the save
  /// button on this window rather than leaving the choice to be regretted.
  final bool dirty;

  /// How the update will be applied (K-297), which is what this window is
  /// really about: a swap and a restart, an installer and a restart, or a file
  /// handed to Flatpak while Lumit stays open.
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
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 460,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(
              quits
                  ? l10n.updateRestartToFinish
                  : l10n.updateDownloaded(version),
              style: t.bodyPrimary,
            ),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(
              switch (delivery) {
                // The Chrome-shaped one: no installer, no questions, a restart
                // that lands in the new version a second later.
                UpdateDelivery.inPlace => l10n.updateReadyInPlace(version) +
                    (dirty ? ' ${l10n.updateUnsavedChanges}' : ''),
                UpdateDelivery.installer => l10n.updateReadyInstaller(version) +
                    (dirty ? ' ${l10n.updateUnsavedChanges}' : ''),
                // Inside the sandbox the files are not ours to replace, so the
                // bundle is handed over and Flatpak does the rest.
                UpdateDelivery.flatpakBundle =>
                  l10n.updateReadyFlatpak(version),
              },
              style: t.small.copyWith(color: t.textMuted),
            ),
          ),
          const SizedBox(height: 14),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            // A Wrap rather than a Row: with unsaved work there are three
            // buttons and one of them is a whole sentence, which overflowed the
            // window and pushed Save and restart off the right edge. Wrapping
            // holds however wide the UI scale makes these labels, rather than
            // depending on a width that happens to fit at 100%.
            child: Wrap(
              alignment: WrapAlignment.end,
              spacing: 8,
              runSpacing: 8,
              children: [
                HouseButton(
                  key: const ValueKey('update-restart-later'),
                  small: true,
                  frameless: true,
                  onPressed: () => onChoose(_RestartAnswer.later),
                  child: Text(l10n.updateLater, style: t.small),
                ),
                if (quits && dirty) ...[
                  HouseButton(
                    key: const ValueKey('update-restart-now'),
                    small: true,
                    frameless: true,
                    onPressed: () => onChoose(_RestartAnswer.restart),
                    child:
                        Text(l10n.updateRestartWithoutSaving, style: t.small),
                  ),
                  HouseButton(
                    key: const ValueKey('update-save-restart'),
                    small: true,
                    // The default (K-319): Enter takes the safe restart — the
                    // one that saves.
                    primary: true,
                    autofocus: true,
                    onPressed: () => onChoose(_RestartAnswer.saveAndRestart),
                    child: Text(l10n.updateSaveAndRestart, style: t.small),
                  ),
                ] else
                  HouseButton(
                    key: const ValueKey('update-restart-now'),
                    small: true,
                    primary: true,
                    autofocus: true,
                    onPressed: () => onChoose(_RestartAnswer.restart),
                    child: Text(
                        quits ? l10n.updateRestartNow : l10n.updateShowTheFile,
                        style: t.small),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 10),
        ],
      ),
    );
  }
}
