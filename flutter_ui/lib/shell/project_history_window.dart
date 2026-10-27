// The Project history window: a project's earlier saves, and the ways back
// to one.
//
// With Lumit Pro every save of a project is kept on Lumit's server as a
// version, and so is an autosave now and then between saves. This window
// lists them newest first, shows the one that is chosen with what changed
// in it, and offers four things to do with it: open it as a copy, bring one
// composition back from it, put it back whole, or give it a name so it is
// kept for good. The foot of the window has the two that decide, open a
// copy and put back. A row's own menu holds all four, and deleting.
//
// It is Lumit's own surfaces and type. The sweep marks Pro and a backup
// that is up to date, and nothing else.
//
// The words for a backup's state live here too, with the mark the bottom
// strip shows for the open project, and Save a version, which saves and
// names in one go.

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:intl/intl.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/project_row_frb.dart' show relinkFootage;
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../src/rust/api/footage.dart';
import '../src/rust/api/project_item.dart';
import '../state/account.dart';
import '../state/backup.dart';
import '../state/file_dialogs.dart';
import '../state/project_changes.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart' show monoSlotWidth;
import 'backups_window.dart' show showBackups;
import 'bring_composition_frb.dart';
import 'dialog_frame.dart';
import 'menu_bar_frb.dart' show saveProjectFrb;
import 'pro_window.dart';
import 'profile_chip.dart';
import 'settings_account.dart' show askBefore;
import 'settings_window_frb.dart' show SettingsPage, showSettingsWindowFrb;
import 'sign_in_window.dart';
import 'welcome_frb.dart' show projectDisplayName;

const double projectHistoryWidth = 800;

/// The room a backup window gives its list, and the little it takes when
/// all it has is a line to say.
const double backupBodyHeight = 440;
const double _lineHeight = 132;

const double _listWidth = 340;
const double _rowHeight = 52;
const double _dayHeight = 28;

/// An autosave nobody has named gets a row this tall, and no picture.
const double _quietRowHeight = 28;

/// The strip over the list that says which versions it shows.
const double _listHeadHeight = 34;

/// The air round the chosen version's picture and words: the inset every
/// other edge of the window takes.
const double _detailInset = dialogPadding;

/// The line under the chosen version's picture that says what it shows.
const double _captionHeight = 24;

/// A version larger than this is not read to say what changed in it.
// ponytail: both versions are fetched whole to be compared, which is too
// much to do on a click for a packed project. Keep a summary beside each
// version when it is sent if those want the line too.
const int _mostCompared = 32 * 1024 * 1024;

/// Where the rail runs down the list, and how big a version's dot on it is.
const double _railAt = 18;
const double _dot = 7;

// --- Words ----------------------------------------------------------------

/// What a refusal is called in the person's own language.
String backupErrorText(CloudError e) => switch (e.code) {
      'pro_required' => l10n.backupNeedsPro,
      'storage_full' => l10n.backupErrorStorageFull,
      'too_large' => l10n.backupErrorTooLarge,
      'too_many_projects' => l10n.backupErrorTooMany,
      'not_found' => l10n.backupErrorGone,
      'last_version' => l10n.backupErrorLastVersion,
      'bad_hash' => l10n.backupErrorDownload,
      'unwritable' => l10n.backupErrorWrite,
      'signed_out' => l10n.backupErrorSignedOut,
      _ => cloudErrorText(e),
    };

String backupSize(int bytes) => writtenBytes(bytes);

String backupTime(DateTime at) {
  try {
    return DateFormat.Hm(l10n.localeName).format(at);
  } catch (_) {
    // A language whose times are not loaded gets the plain form.
    return DateFormat.Hm().format(at);
  }
}

bool _sameDay(DateTime a, DateTime b) =>
    a.year == b.year && a.month == b.month && a.day == b.day;

/// Today, Yesterday, or the date written out, with its year only when it
/// is not this one.
String backupDay(DateTime at) {
  final now = DateTime.now();
  if (_sameDay(at, now)) return l10n.backupDayToday;
  if (_sameDay(at, now.subtract(const Duration(days: 1)))) {
    return l10n.backupDayYesterday;
  }
  return writtenDay(at);
}

String backupWhen(DateTime at) =>
    l10n.backupDayTime(backupDay(at), backupTime(at));

/// The same with the date always written out, for the middle of a sentence.
String backupWhenWritten(DateTime at) =>
    l10n.backupDayTime(writtenDate(at), backupTime(at));

/// The same as the list shows it, for the middle of a sentence: today, 01:11.
String backupWhenWithin(DateTime at) {
  final now = DateTime.now();
  final day = _sameDay(at, now)
      ? l10n.backupDayTodayWithin
      : _sameDay(at, now.subtract(const Duration(days: 1)))
          ? l10n.backupDayYesterdayWithin
          : writtenDate(at);
  return l10n.backupDayTime(day, backupTime(at));
}

/// How the state of a backup is to be read: there is nothing to do, there
/// is something to know, or the save was refused.
enum BackupTone { quiet, told, refused }

/// How the open project's backup stands, in words, and how they are to be
/// read. Waiting is quiet, since Lumit tries again by itself.
(String, BackupTone) backupStatusWords(BackupState backup) {
  if (backup.newer case final newer?) {
    return (l10n.backupStatusNewer(newer.device), BackupTone.told);
  }
  final at = backup.backedUp;
  return switch (backup.status) {
    BackupStatus.off => ('', BackupTone.quiet),
    BackupStatus.backedUp => (
        at == null
            ? l10n.backupStatusBackedUp
            : _sameDay(at, DateTime.now())
                ? l10n.backupStatusBackedUpAt(backupTime(at))
                : l10n.backupStatusBackedUpOn(writtenDate(at)),
        BackupTone.quiet
      ),
    BackupStatus.sending => (l10n.backupStatusSending, BackupTone.quiet),
    BackupStatus.waiting => (l10n.backupStatusWaiting, BackupTone.quiet),
    BackupStatus.failed => (
        backup.error == null
            ? l10n.backupStatusFailed
            : backupErrorText(backup.error!),
        BackupTone.refused
      ),
    BackupStatus.tooLarge => (l10n.backupStatusTooLarge, BackupTone.refused),
  };
}

/// The version that [version] is set against to say what changed in it, out
/// of [versions], newest first. That is the one its machine had when it made
/// this one, so with two machines saving in turn a version is never shown
/// the other machine's changes as its own. Where that one has been thinned
/// away, or was never said, it is the one before it in the list. Null for a
/// project's first.
BackupVersion? backupComparedWith(
    BackupVersion version, List<BackupVersion> versions) {
  final at = versions.indexWhere((v) => v.n == version.n);
  if (at < 0) return null;
  final base = version.base;
  return versions.where((v) => v.n == base && v.n < version.n).firstOrNull ??
      (at + 1 < versions.length ? versions[at + 1] : null);
}

/// What changed in a version since the one before it, one kind of change to
/// a line: how many, and the words that follow the number, as in 3 and
/// "layers changed in Titles". Empty when nothing a person would call a
/// change did.
List<(int, String)> backupChangeLines(ProjectChanges changes) {
  // At most three compositions by name, and the rest as a number.
  String where(List<String> names) => switch (names.length) {
        0 => '',
        1 => names[0],
        2 => l10n.backupChangeWhere2(names[0], names[1]),
        3 => l10n.backupChangeWhere3(names[0], names[1], names[2]),
        _ => l10n.backupChangeWhereMore(
            names[0], names[1], names[2], names.length - 3),
      };
  return [
    (changes.compsAdded, l10n.backupChangeCompsAdded),
    (changes.compsRemoved, l10n.backupChangeCompsRemoved),
    (changes.compsRenamed, l10n.backupChangeCompsRenamed),
    (
      changes.layersAdded,
      (int n) => l10n.backupChangeLayersAdded(n, where(changes.addedIn))
    ),
    (
      changes.layersRemoved,
      (int n) => l10n.backupChangeLayersRemoved(n, where(changes.removedIn))
    ),
    (
      changes.layersChanged,
      (int n) => l10n.backupChangeLayersChanged(n, where(changes.changedIn))
    ),
    (changes.effectsAdded, l10n.backupChangeEffectsAdded),
    (changes.effectsRemoved, l10n.backupChangeEffectsRemoved),
    (changes.footageAdded, l10n.backupChangeFootageAdded),
    (changes.footageRemoved, l10n.backupChangeFootageRemoved),
  ]
      .where((line) => line.$1 > 0)
      .map((line) => (line.$1, line.$2(line.$1)))
      .toList();
}

// --- Shared pieces --------------------------------------------------------

/// The state of a backup as a small mark, one to a state: a tick in the
/// sweep when it is backed up, a ring while a save is going, a loop while
/// one waits to go, a warning when one was refused, and an arrow down when
/// another machine has saved a version this one has not got.
class BackupGlyph extends StatelessWidget {
  final BackupState backup;
  final double size;

  /// The project has changes that are not saved. The tick is then about a
  /// file that is behind the work, so it is drawn quietly and not in the
  /// sweep.
  final bool unsaved;
  const BackupGlyph(this.backup,
      {super.key, this.size = 12, this.unsaved = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget icon(String which, Color colour) =>
        glyph.LumitIcon(which, size: size, colour: colour);
    final mark = backup.newer != null
        ? icon(LumitIcons.import, t.textPrimary)
        : switch (backup.status) {
            BackupStatus.sending => const SyncRing(),
            BackupStatus.backedUp when unsaved =>
              icon(LumitIcons.tick, t.textSecondary),
            BackupStatus.backedUp => ShaderMask(
                blendMode: BlendMode.srcIn,
                shaderCallback: (bounds) => proSweep.createShader(bounds),
                child: icon(LumitIcons.tick, t.textPrimary),
              ),
            BackupStatus.failed ||
            BackupStatus.tooLarge =>
              icon(LumitIcons.missing, t.warning),
            BackupStatus.waiting ||
            BackupStatus.off =>
              icon(LumitIcons.loop, t.textSecondary),
          };
    return SizedBox(width: size, height: size, child: mark);
  }
}

/// The open project's backup on the bottom strip, beside whether it is
/// saved. Nothing at all without Pro, or for a project with no history.
/// A press opens the Project history window.
class BackupMark extends StatefulWidget {
  /// Dresses the mark the way the strip dresses its other readouts.
  final Widget Function(Widget child) wrap;

  /// Whether the project has unsaved changes, which the strip knows.
  final bool unsaved;
  const BackupMark({super.key, required this.wrap, this.unsaved = false});

  @override
  State<BackupMark> createState() => _BackupMarkState();
}

class _BackupMarkState extends State<BackupMark> {
  late final LumitUiState _ui = context.read<LumitUiState>();

  void _show() {
    if (mounted) unawaited(showProjectHistory(context));
  }

  @override
  void initState() {
    super.initState();
    // The strip is up for as long as the window is, so it is where a notice
    // about the open project's history leads.
    _ui.backup.showHistory = _show;
  }

  @override
  void dispose() {
    if (_ui.backup.showHistory == _show) _ui.backup.showHistory = null;
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: Listenable.merge([_ui.backup, _ui.account]),
        builder: (context, _) {
          final backup = _ui.backup;
          final (words, _) = backupStatusWords(backup);
          if (!_ui.account.pro || backup.id == null || words.isEmpty) {
            return const SizedBox.shrink();
          }
          return widget.wrap(
            Padding(
              padding: const EdgeInsets.only(left: 2),
              child: LumitTooltip(
                message: words,
                child: HouseButton(
                  key: const ValueKey('status-backup'),
                  frameless: true,
                  small: true,
                  padding: const EdgeInsets.symmetric(horizontal: 4),
                  onPressed: _show,
                  child: BackupGlyph(backup, unsaved: widget.unsaved),
                ),
              ),
            ),
          );
        },
      );
}

/// A version's picture, or a quiet well where it has none.
class BackupStill extends StatefulWidget {
  final BackupState backup;
  final String id;
  final BackupVersion? version;
  final double? width, height;

  /// Show the whole picture inside the well, and not as much of it as fills
  /// the well. For the one large picture a version is chosen by, where a
  /// tall or square composition would otherwise show as a slice. That one
  /// sizes itself, and has the line under it that says what it shows.
  final bool whole;

  const BackupStill(this.backup, this.id, this.version,
      {super.key, this.width, this.height, this.whole = false});

  @override
  State<BackupStill> createState() => _BackupStillState();
}

class _BackupStillState extends State<BackupStill> {
  Uint8List? _png;

  @override
  void initState() {
    super.initState();
    _fetch();
  }

  @override
  void didUpdateWidget(BackupStill old) {
    super.didUpdateWidget(old);
    if (old.id != widget.id || old.version?.n != widget.version?.n) {
      _png = null;
      _fetch();
    }
  }

  /// Asked for here and never in a build: it is a request to the server the
  /// first time each picture is wanted.
  void _fetch() {
    final version = widget.version;
    if (version == null || !version.thumb) return;
    unawaited(widget.backup.still(widget.id, version.n).then((png) {
      if (mounted && widget.version?.n == version.n) {
        setState(() => _png = png);
      }
    }));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final small = (widget.height ?? 100) < 60;
    final radius = BorderRadius.circular(
        small ? t.tokens.controlRadius : t.tokens.sectionRadius);
    final nothing = Center(
      child: glyph.LumitIcon(LumitIcons.composition,
          size: small ? 14 : 24, colour: t.textDisabled),
    );
    final png = _png;
    final picture = Container(
      width: widget.width,
      height: widget.height,
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(color: t.surface0, borderRadius: radius),
      foregroundDecoration: BoxDecoration(
        borderRadius: radius,
        border: Border.all(color: t.hairline),
      ),
      child: png == null
          ? nothing
          : Image.memory(
              png,
              // A row's picture is decoded small. It is sent large, for the
              // one place it is drawn large, and a list of them whole would
              // hold a couple of megabytes each.
              cacheWidth: switch (widget.width) {
                final width? =>
                  (width * 2 * MediaQuery.devicePixelRatioOf(context)).round(),
                null => null,
              },
              fit: widget.whole ? BoxFit.contain : BoxFit.cover,
              gaplessPlayback: true,
              filterQuality: FilterQuality.medium,
              errorBuilder: (_, __, ___) => nothing,
            ),
    );
    if (!widget.whole) return picture;
    // The large picture, with what it says it shows under it. The line is
    // kept for a picture that does not say, so the words below it stay where
    // they are from one version to the next.
    final shows = png == null ? null : stillCaption(png);
    final mono = dialogMono(t);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        AspectRatio(aspectRatio: 16 / 9, child: picture),
        SizedBox(
          height: _captionHeight,
          child: shows == null
              ? null
              : Row(
                  key: const ValueKey('history-shows'),
                  children: [
                    Flexible(
                      child: Text(shows.$1,
                          style: mono,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis),
                    ),
                    Text('${shows.$1.isEmpty ? '' : ' · '}${shows.$2}',
                        style: mono),
                  ],
                ),
        ),
      ],
    );
  }
}

/// One line in the middle of a backup window, with a way to ask again when
/// the line is that the server was not reached.
Widget backupLine(LumitTheme t, String words, {VoidCallback? retry}) =>
    SizedBox(
      height: _lineHeight,
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Text(words, style: t.body.copyWith(color: t.textSecondary)),
          if (retry != null) ...[
            const SizedBox(height: 14),
            SizedBox(
              height: dialogFooterButton,
              child: HouseButton(
                key: const ValueKey('backup-retry'),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                onPressed: retry,
                child: Text(l10n.backupTryAgain),
              ),
            ),
          ],
        ],
      ),
    );

/// The ring a backup window shows while it waits for its list, in the room
/// the list will take.
const Widget backupWaiting = SizedBox(
  height: backupBodyHeight,
  child: Center(child: SizedBox(width: 20, height: 20, child: SyncRing())),
);

/// The same ring in the room one line takes, for a window that is more
/// likely to have a line to say than a list.
const Widget backupWaitingLine = SizedBox(
  height: _lineHeight,
  child: Center(child: SizedBox(width: 20, height: 20, child: SyncRing())),
);

/// The strip under a backup window's title: the badge, a line, and whatever
/// control the window has for it.
Widget backupStrip(LumitTheme t,
        {required bool preview,
        bool badge = true,
        Widget? mark,
        String words = '',
        BackupTone tone = BackupTone.quiet,
        List<Widget> trailing = const []}) =>
    Container(
      height: 38,
      padding: const EdgeInsets.symmetric(horizontal: dialogPadding),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      child: Row(
        children: [
          // No badge for an account that has not got Pro.
          if (badge) ...[
            ProBadge(preview: preview),
            const SizedBox(width: 12),
          ],
          if (mark != null) ...[mark, const SizedBox(width: 7)],
          Expanded(
            child: Text(
              words,
              key: const ValueKey('backup-strip-words'),
              style: t.small.copyWith(
                  color: switch (tone) {
                BackupTone.quiet => t.textSecondary,
                BackupTone.told => t.textPrimary,
                BackupTone.refused => t.warning,
              }),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          ...trailing,
        ],
      ),
    );

/// The strip for an account whose Lumit Pro has ended and which still has
/// what it backed up: it says that saves are no longer backed up, in the
/// place every other reason backup has stopped is said, and offers the way
/// back to Pro. [trailing] comes after that button.
Widget backupEndedStrip(BuildContext context, LumitTheme t,
        {List<Widget> trailing = const []}) =>
    backupStrip(
      t,
      preview: false,
      badge: false,
      mark: glyph.LumitIcon(LumitIcons.missing, size: 12, colour: t.warning),
      words: l10n.backupProEnded,
      tone: BackupTone.refused,
      trailing: [
        SizedBox(
          height: dialogControlHeight,
          child: HouseButton(
            key: const ValueKey('backup-ended-pro'),
            small: true,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            onPressed: () => unawaited(showProWindow(context)),
            child: Text(l10n.menuLumitPro,
                style: t.small.copyWith(color: t.textPrimary)),
          ),
        ),
        ...trailing,
      ],
    );

// --- Putting a version back -----------------------------------------------

/// Make [version] the open project again. True when it was.
///
/// Nothing is written over until the server holds the project as it stands.
/// Unsaved changes are saved first, that save is sent, and if either does
/// not happen this stops with the file untouched. So what was there before
/// is always a version to go back to. A file the server already holds as a
/// version makes no new one on the way.
///
/// [onReplaced] is called once the file has been written over, just before
/// it is opened again.
Future<bool> putBackVersion(
    LumitState app, LumitUiState ui, BackupVersion version,
    {VoidCallback? onReplaced}) async {
  final backup = ui.backup;
  final project = app.project;
  final id = backup.id;
  final path = project?.path();
  if (project == null || id == null || path == null) return false;
  if (project.isDirty()) {
    await saveProjectFrb(app, ui);
    // The save says so itself when it fails.
    if (project.isDirty()) return false;
  }
  if (await backup.versionOnDisk() == null) {
    app.postNotice(l10n.backupPutBackUnsent, error: true);
    return false;
  }
  try {
    await backup.fetch(id, version, path);
  } on CloudError catch (e) {
    app.postNotice(backupErrorText(e), error: true);
    return false;
  }
  // The file is the old version's bytes now. Saying so is what records that
  // version as the newest, and the server has the bytes already.
  await backup.recordWrittenFrom(version);
  onReplaced?.call();
  await app.openProject(path, recover: false);
  await noticeMissingFootage(app, ui);
  return true;
}

// --- Footage that is not there ----------------------------------------------

/// Say how much of the open project's footage is missing, after it has come
/// out of backup. A backup is the project file, so on a machine that has not
/// got the footage this is the first thing to know. A press on the notice
/// picks the first missing file in the Project panel and asks where it is,
/// as Relink does on its row.
Future<void> noticeMissingFootage(LumitState app, LumitUiState ui) async {
  final footage = <FootageReference>[];
  void gather(ItemReference item) {
    switch (item) {
      case ItemReference_Footage(:final field0):
        footage.add(field0);
      case ItemReference_Folder(:final field0):
        field0.getChildren().forEach(gather);
      default:
    }
  }

  final missing = <FootageReference>[];
  final project = app.project;
  try {
    project?.getItems().forEach(gather);
    for (final file in footage) {
      if (await file.getStatus() == LumitMediaStatus.missing) {
        missing.add(file);
      }
    }
  } catch (_) {
    // The project closed while its files were being looked for.
    return;
  }
  if (missing.isEmpty) return;
  app.postNotice(l10n.backupFootageMissing(missing.length),
      error: true,
      onTap: () {
        // Another project may have been opened under the notice.
        if (!identical(app.project, project)) return;
        ui.projectPickRequest.value = ItemReference.footage(missing.first);
        // The panel hears of the edit as it does of any made outside it.
        unawaited(relinkFootage(missing.first));
      });
}

// --- Saving a version with a name -------------------------------------------

/// File ▸ Save a version: ask for a name, save the project, and give the
/// version that save made the name. A named version is kept for good.
///
/// Without Lumit Pro there is nothing to name, so the Project history window
/// opens in its place, which says what this comes with.
///
/// [picker] is the seam `saveProjectFrb` takes, for a test.
Future<void> saveVersionFrb(
    BuildContext context, LumitState app, LumitUiState ui,
    {Future<String?> Function()? picker}) async {
  if (!ui.account.pro) return showProjectHistory(context);
  final name = await askProfileName(context,
      title: l10n.backupSaveVersionTitle,
      confirm: l10n.save);
  if (name == null) return;
  await saveNamedVersion(app, ui, name, picker: picker);
}

/// The save and the naming, without the question. True when the version has
/// its name.
Future<bool> saveNamedVersion(LumitState app, LumitUiState ui, String name,
    {Future<String?> Function()? picker}) async {
  await saveProjectFrb(app, ui, picker: picker);
  final project = app.project;
  // Cancelled at the picker, or the save failed and said so itself.
  if (project == null || project.path() == null || project.isDirty()) {
    return false;
  }
  final backup = ui.backup;
  try {
    final n = await backup.versionOnDisk();
    final id = backup.id;
    if (n == null || id == null) {
      app.postNotice(l10n.backupVersionUnsent, error: true);
      return false;
    }
    await backup.nameVersion(id, n, name);
    app.postNotice(l10n.backupVersionSaved(name));
    return true;
  } on CloudError catch (e) {
    app.postNotice(backupErrorText(e), error: true);
    return false;
  }
}

// --- The window -----------------------------------------------------------

/// Open the Project history window, for the open project or for [of], a
/// backed-up project that is not the one open.
Future<void> showProjectHistory(BuildContext context, {BackupProject? of}) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  return showLumitModal<void>(
    context: context,
    id: 'project-history',
    builder: (close) =>
        _History(app: app, ui: ui, other: of, close: () => close(null)),
  );
}

/// Ask before a version is put back. Nothing is lost by it, so this takes
/// the form of the prompt that names a version and not of the one that
/// deletes: the filled button is the one that goes ahead, and Enter presses
/// it.
Future<bool> _askPutBack(BuildContext context, String question,
    {String? yes}) async {
  final sure = await showLumitModal<bool>(
    context: context,
    builder: (close) => FloatSurface(
      width: 320,
      child: Padding(
        padding: const EdgeInsets.all(14),
        child: Builder(builder: (context) {
          final t = ThemeScope.of(context).theme;
          return Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(question, style: t.bodyPrimary.copyWith(height: 1.4)),
              const SizedBox(height: 6),
              Text(l10n.backupPutBackNote, style: t.small),
              const SizedBox(height: 12),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  HouseButton(
                    key: const ValueKey('put-back-no'),
                    small: true,
                    frameless: true,
                    onPressed: () => close(false),
                    child: Text(l10n.cancel),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('put-back-yes'),
                    small: true,
                    primary: true,
                    autofocus: true,
                    onPressed: () => close(true),
                    child: Text(yes ?? l10n.backupPutBack),
                  ),
                ],
              ),
            ],
          );
        }),
      ),
    ),
  );
  return sure ?? false;
}

class _History extends StatefulWidget {
  final LumitState app;
  final LumitUiState ui;
  final BackupProject? other;
  final VoidCallback close;

  const _History(
      {required this.app,
      required this.ui,
      required this.other,
      required this.close});

  @override
  State<_History> createState() => _HistoryState();
}

class _HistoryState extends State<_History> {
  /// Where the project's file is on this machine, if it has one, and what
  /// the project is called. Read once: both are questions for the engine or
  /// the disk.
  String? _path;
  String _name = '';

  BackupProject? _project;
  List<BackupVersion>? _versions;
  CloudError? _error;
  int? _chosen;

  /// The hash of the open project's file as it is on disk, read each time
  /// the list is asked for. It says which version the file is, or that it
  /// is none of them.
  String? _onDisk;

  /// Only versions somebody has named are listed.
  bool _namedOnly = false;

  /// Something asked for from here is under way, the word for it when it
  /// is a wait worth naming, and what the last thing asked for came back
  /// with when it failed.
  bool _busy = false;
  String _doing = '';
  String? _said;

  /// The first rows come in one after another, until the list is scrolled.
  bool _fresh = true;
  bool _asking = false;

  /// How many versions had been sent when the list was last asked for.
  int _sent = 0;

  /// What each project file holds, by its hash, and what changed in each
  /// version since the one before it. Both are kept while the window is
  /// open, so a version looked at twice is worked out once.
  final Map<String, Future<ProjectShape?>> _shapes = {};
  final Map<int, List<(int, String)>> _whatChanged = {};

  /// The keys' way in, and the scrolling of the list and of the words under
  /// the picture.
  final _focus = FocusNode(debugLabel: 'project history');
  final _scroll = ScrollController();
  final _detailScroll = ScrollController();

  /// Whether the words under the picture run on below what is shown.
  final _moreWords = ValueNotifier(false);

  /// Where each row sits in the list and how tall it is, by version number,
  /// and where each day's heading sits. Noted as the list is built, for the
  /// heading held at its top and for keeping the chosen row in view.
  Map<int, (double, double)> _rowAt = const {};
  List<(double, String)> _dayAt = const [];

  /// The rows the keys move among, in the list's order, each as a version:
  /// a version's own row, or for a run of autosaves folded to one row, the
  /// newest of the run. A run is known by that newest version. [_folds] are
  /// the runs that are folded rows, [_unfolded] the ones that were opened,
  /// [_runOf] the run each row of an open run belongs to, and [_foldAt] the
  /// folded row the keys are on.
  List<BackupVersion> _rowed = const [];
  Set<int> _folds = const {};
  Map<int, int> _runOf = const {};
  final Set<int> _unfolded = {};
  int? _foldAt;

  /// Runs out a moment after the keys stop stepping through the list.
  Timer? _settle;

  BackupState get _backup => widget.ui.backup;
  AccountState get _account => widget.ui.account;

  /// Whether this is the open project's history.
  bool get _open => widget.other == null;
  String? get _id => widget.other?.id ?? _backup.id;

  @override
  void initState() {
    super.initState();
    String? path;
    try {
      path = _open
          ? widget.app.project?.path()
          : _backup.localPath(widget.other!.id);
    } catch (_) {
      // Closed already.
    }
    _path = path;
    _name = widget.other?.name ??
        (path == null ? '' : projectDisplayName(path));
    _sent = _backup.versionsSent;
    _backup.addListener(_changed);
    _account.addListener(_changed);
    unawaited(_load());
  }

  @override
  void dispose() {
    _settle?.cancel();
    _backup.removeListener(_changed);
    _account.removeListener(_changed);
    _focus.dispose();
    _scroll.dispose();
    _detailScroll.dispose();
    _moreWords.dispose();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    // A save or an autosave that has just gone is a version now, and signing
    // in from the Lumit Pro window is the first chance to ask at all.
    final sent = _backup.versionsSent != _sent;
    _sent = _backup.versionsSent;
    if (sent || (_versions == null && _error == null)) unawaited(_load());
    setState(() {});
  }

  Future<void> _load() async {
    final id = _id;
    if (id == null || !_account.signedIn || _asking) return;
    _asking = true;
    try {
      final onDisk = _open ? _backup.onDisk() : Future<String?>.value();
      final (project, versions) = await _backup.history(id);
      final file = await onDisk;
      if (!mounted) return;
      setState(() {
        _project = project;
        _versions = versions;
        _onDisk = file;
        _error = null;
        // Which version comes before which may have changed.
        _whatChanged.clear();
        if (!versions.any((v) => v.n == _chosen)) {
          _chosen = _listed(versions).firstOrNull?.n;
        }
      });
      final chosen = versions.where((v) => v.n == _chosen).firstOrNull;
      if (chosen != null) unawaited(_workOut(chosen));
    } on CloudError catch (e) {
      if (!mounted) return;
      setState(() {
        // Not there is a history with nothing in it yet.
        _versions = e.status == 404 ? const [] : _versions;
        _error = e.status == 404 ? null : e;
      });
    } finally {
      _asking = false;
    }
  }

  /// The versions the list shows.
  List<BackupVersion> _listed(List<BackupVersion> versions) => _namedOnly
      ? [
          for (final v in versions)
            if (v.note.isNotEmpty) v,
        ]
      : versions;

  /// The version that is the open project's file as it was last saved: the
  /// newest one whose bytes are the file on disk. An autosave sent since
  /// sits above it. There is none to mark when the file has been saved
  /// again and that save has not gone, or in another project's history.
  int? _lastSave(List<BackupVersion> versions) => _open && _onDisk != null
      ? versions.where((v) => v.hash == _onDisk).firstOrNull?.n
      : null;

  /// Whether the open project's file is none of these versions: saved since
  /// the last one was sent, with that save still to go or never to. Not
  /// said in the moment a save is on its way.
  bool _ahead(List<BackupVersion> versions) =>
      _open &&
      _onDisk != null &&
      _backup.status != BackupStatus.sending &&
      _lastSave(versions) == null;

  /// Whether [version] gets a whole row, with its picture. An autosave
  /// nobody has named is one quiet line, but not when it is the newest
  /// version of all: after a crash that is the row the person came for.
  bool _whole(BackupVersion version, List<BackupVersion> all) =>
      !version.quiet || version.n == all.first.n;

  /// Whether [version] can be put back. Not for a project kept out of
  /// backup, where there would be nowhere to keep the project as it stands
  /// first, and not the version the file on disk already is. Without Pro
  /// it goes ahead when the file here is already a version, and says so
  /// when it is not.
  bool _canPutBack(BackupVersion version) =>
      _open &&
      _backup.on &&
      !_backup.out &&
      version.hash != (_onDisk ?? _backup.held);

  /// Choose [version]. Stepping through the list with the keys waits until
  /// they stop before anything is fetched to say what changed, since each
  /// row passed on the way would fetch two project files.
  void _choose(BackupVersion version, {bool stepping = false}) {
    setState(() {
      _chosen = version.n;
      // On a folded run the choice is its row, and what is shown is its
      // newest autosave.
      _foldAt = stepping && _folds.contains(version.n) ? version.n : null;
    });
    if (_detailScroll.hasClients) _detailScroll.jumpTo(0);
    _settle?.cancel();
    if (stepping) {
      _settle = Timer(const Duration(milliseconds: 300),
          () => unawaited(_workOut(version)));
    } else {
      unawaited(_workOut(version));
    }
  }

  void _showNamedOnly(bool on) {
    if (on == _namedOnly) return;
    setState(() {
      _namedOnly = on;
      _foldAt = null;
      final shown = _listed(_versions ?? const []);
      if (!shown.any((v) => v.n == _chosen)) {
        _chosen = shown.firstOrNull?.n ?? _chosen;
      }
    });
    if (_scroll.hasClients) _scroll.jumpTo(0);
    final chosen = _versions?.where((v) => v.n == _chosen).firstOrNull;
    if (chosen != null) unawaited(_workOut(chosen));
  }

  /// Up and down move the choice, Page Up and Page Down move it by as much
  /// as the list shows at once, Home and End take it to the ends, and Enter
  /// is the window's filled button. The choice moves among the rows there
  /// are, and a run of autosaves folded to one row is one of them: Right or
  /// Enter opens it, and Left on a row of an open run folds it again.
  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is KeyUpEvent || _busy) return KeyEventResult.ignored;
    final shown = _rowed;
    if (shown.isEmpty) return KeyEventResult.ignored;
    final at = shown.indexWhere((v) => v.n == _chosen);
    final to = switch (event.logicalKey) {
      LogicalKeyboardKey.arrowUp => math.max(0, at - 1),
      LogicalKeyboardKey.arrowDown => math.min(shown.length - 1, at + 1),
      LogicalKeyboardKey.pageUp => _paged(shown, at, -1),
      LogicalKeyboardKey.pageDown => _paged(shown, at, 1),
      LogicalKeyboardKey.home => 0,
      LogicalKeyboardKey.end => shown.length - 1,
      _ => null,
    };
    if (to != null) {
      if (to != at) {
        _choose(shown[to], stepping: true);
        _reveal(shown[to].n);
      }
      return KeyEventResult.handled;
    }
    final enter = event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.numpadEnter;
    final fold = _foldAt;
    if (fold != null) {
      if (!enter && event.logicalKey != LogicalKeyboardKey.arrowRight) {
        return KeyEventResult.ignored;
      }
      // Opened, with the choice on its first autosave, which is the one the
      // folded row was showing.
      setState(() {
        _unfolded.add(fold);
        _foldAt = null;
      });
      return KeyEventResult.handled;
    }
    final run = _runOf[_chosen];
    if (event.logicalKey == LogicalKeyboardKey.arrowLeft && run != null) {
      final newest = _versions?.where((v) => v.n == run).firstOrNull;
      if (newest == null) return KeyEventResult.ignored;
      setState(() {
        _unfolded.remove(run);
        _chosen = run;
        _foldAt = run;
      });
      unawaited(_workOut(newest));
      WidgetsBinding.instance.addPostFrameCallback((_) => _reveal(run));
      return KeyEventResult.handled;
    }
    if (enter && event is KeyDownEvent && at >= 0 && _canPutBack(shown[at])) {
      unawaited(_putBack(shown[at]));
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// The row a screenful above or below row [at] of [shown].
  int _paged(List<BackupVersion> shown, int at, int step) {
    if (at < 0 || !_scroll.hasClients) return math.max(0, at);
    final page = _scroll.position.viewportDimension - _dayHeight;
    final from = _rowAt[shown[at].n]?.$1 ?? 0;
    var to = at;
    for (var i = at + step; i >= 0 && i < shown.length; i += step) {
      if (((_rowAt[shown[i].n]?.$1 ?? from) - from).abs() > page) break;
      to = i;
    }
    return to;
  }

  /// Scroll the list as little as brings version [n] into view, clear of
  /// the day held at its top.
  void _reveal(int n) {
    final place = _rowAt[n];
    if (place == null || !_scroll.hasClients) return;
    final (top, height) = place;
    final position = _scroll.position;
    double? to;
    if (top < position.pixels + _dayHeight) {
      to = top - _dayHeight;
    } else if (top + height > position.pixels + position.viewportDimension) {
      to = top + height - position.viewportDimension;
    }
    if (to != null) _scroll.jumpTo(to.clamp(0, position.maxScrollExtent));
  }

  /// What the file of [version] holds. Fetched when it is first wanted and
  /// read off the UI thread, and null when it can't be read.
  Future<ProjectShape?> _shapeOf(BackupVersion version) {
    final id = _id;
    if (id == null || version.size > _mostCompared) return Future.value();
    return _shapes[version.hash] ??= () async {
      try {
        return await readProjectShape(
            await _backup.bytes(id, version), version.hash);
      } on CloudError {
        // Not reached. Asked for again the next time it is wanted.
        unawaited(_shapes.remove(version.hash));
        return null;
      }
    }();
  }

  /// Work out what changed in [version], from the two files. Nothing is
  /// said for the first version, or when either file can't be read: nothing
  /// is better than something wrong.
  Future<void> _workOut(BackupVersion version) async {
    final versions = _versions;
    if (!mounted ||
        versions == null ||
        _whatChanged.containsKey(version.n)) {
      return;
    }
    final before = backupComparedWith(version, versions);
    if (before == null) return;
    var lines = const <(int, String)>[];
    if (before.hash != version.hash) {
      final [was, now] =
          await Future.wait([_shapeOf(before), _shapeOf(version)]);
      if (was == null || now == null) return;
      lines = backupChangeLines(changesBetween(was, now));
    }
    if (mounted) setState(() => _whatChanged[version.n] = lines);
  }

  /// Run something the person asked for, with the buttons held meanwhile
  /// and its refusal said in the strip. [saying] is the word for the wait
  /// when it may be a long one: a version's file can be 100 MB.
  Future<void> _run(Future<void> Function() what, {String saying = ''}) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _doing = saying;
      _said = null;
    });
    try {
      await what();
    } on CloudError catch (e) {
      _said = backupErrorText(e);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// What a question calls [version]: its name when it has one, and
  /// otherwise when it was saved, the way the list says it.
  String _asked(BackupVersion version,
          {required String Function(String) named,
          required String Function(String) dated}) =>
      version.note.isEmpty
          ? dated(backupWhenWithin(version.created))
          : named(version.note);

  /// The keys go back to the list once a question asked over it has gone.
  void _answered() {
    if (mounted) _focus.requestFocus();
  }

  Future<void> _giveName(BackupVersion version) async {
    final name = await askProfileName(context,
        title: version.note.isEmpty
            ? l10n.backupNameTitle
            : l10n.backupRenameVersionTitle,
        suggested: version.note,
        confirm: l10n.save);
    _answered();
    if (name == null) return;
    await _run(() async {
      await _backup.nameVersion(_id!, version.n, name);
      await _load();
    });
  }

  /// Take the name off [version], which is then thinned out with age like
  /// any other.
  Future<void> _removeName(BackupVersion version) => _run(() async {
        await _backup.nameVersion(_id!, version.n, '');
        await _load();
      });

  Future<void> _delete(BackupVersion version) async {
    final sure = await askBefore(
        context,
        _asked(version,
            named: l10n.backupDeleteNamedAsk,
            dated: l10n.backupDeleteVersionAsk),
        l10n.delete,
        note: l10n.backupDeleteVersionNote);
    _answered();
    if (!sure) return;
    await _run(() async {
      await _backup.deleteVersion(_id!, version);
      await _load();
    });
  }

  /// Fetch [version] as a file of its own and open it. It goes beside the
  /// project's file so its footage is found, and where this machine has no
  /// such file the person says where.
  Future<void> _copy(BackupVersion version) async {
    final path = _path;
    final file = '$_name '
        '(${DateFormat('yyyy-MM-dd HH.mm').format(version.created)}).lum';
    var target = path == null
        ? await pickProjectSaveLocation(
            suggestedName: file, initialDirectory: _backup.folder)
        : '${File(path).parent.path}${Platform.pathSeparator}$file';
    if (target == null) return;
    if (!target.toLowerCase().endsWith('.lum')) target = '$target.lum';
    final to = target;
    await _run(() async {
      await _backup.fetch(_id!, version, to);
      final (app, ui) = (widget.app, widget.ui);
      widget.close();
      await app.openProject(to);
      await noticeMissingFootage(app, ui);
    }, saying: l10n.backupFetching);
  }

  /// Fetch [version] to a file of its own for as long as it takes to choose
  /// one of its compositions and bring that one into the project as it is
  /// now. The window closes once one has come in, so it is there to see.
  Future<void> _bring(BackupVersion version) async {
    await _run(() async {
      final folder = await Directory.systemTemp.createTemp('lumit-version');
      try {
        final file = '${folder.path}${Platform.pathSeparator}version.lum';
        await _backup.fetch(_id!, version, file);
        if (!mounted) return;
        // In a name the date is written out: Today would be wrong tomorrow.
        final brought = await bringCompositionFrom(context, file,
            earlierSave: true,
            subject: version.note.isEmpty
                ? backupWhen(version.created)
                : version.note,
            source: version.note.isEmpty
                ? backupWhenWritten(version.created)
                : version.note);
        if (brought) widget.close();
      } finally {
        try {
          await folder.delete(recursive: true);
        } catch (_) {
          // Left for the system to clear with the rest of its temp folder.
        }
      }
    }, saying: l10n.backupFetching);
    _answered();
  }

  Future<void> _putBack(BackupVersion version) async {
    final sure = await _askPutBack(
        context,
        _asked(version,
            named: l10n.backupPutBackNamedAsk, dated: l10n.backupPutBackAsk));
    _answered();
    if (!sure) return;
    await _run(() async {
      await putBackVersion(widget.app, widget.ui, version,
          onReplaced: widget.close);
    }, saying: l10n.backupFetching);
  }

  /// Take the newer version another machine saved, from where the window
  /// says there is one. It is what Put back does, asked in words that fit a
  /// version ahead of this file and not behind it.
  Future<void> _openNewer(BackupVersion version) async {
    final when = version.created;
    final sure = await _askPutBack(
        context,
        version.device.isEmpty
            ? l10n.backupPutBackAsk(backupWhenWithin(when))
            : _sameDay(when, DateTime.now())
                ? l10n.backupOpenNewerAskToday(
                    version.device, backupTime(when))
                : l10n.backupOpenNewerAsk(
                    version.device, backupWhenWithin(when)),
        yes: l10n.backupOpenNewerConfirm);
    _answered();
    if (!sure) return;
    await _run(() async {
      await putBackVersion(widget.app, widget.ui, version,
          onReplaced: widget.close);
    }, saying: l10n.backupFetching);
  }

  /// To where room is made when the account's storage is full: the Open
  /// from backup window, which says how much is used and lets a project go.
  /// The file is tried again when that window closes.
  Future<void> _freeUp() async {
    await showBackups(context, full: true);
    if (!mounted) return;
    _backup.resend();
    _answered();
  }

  /// Everything there is to do with [version], for its row's own menu and
  /// for a right click: what the foot of the window offers, live when that
  /// is, and taking a name off or deleting.
  List<Widget> _menu(LumitTheme t, BackupVersion version, VoidCallback close) {
    Widget row(String id, String label, VoidCallback? then) {
      final live = then != null && !_busy;
      return MenuRow(
        key: ValueKey<String>('history-$id-${version.n}'),
        onPressed: () {
          close();
          if (live) then();
        },
        child: Text(label,
            style: live ? null : t.body.copyWith(color: t.textDisabled)),
      );
    }

    return [
      if (_open)
        row('menu-put-back', l10n.backupPutBack,
            _canPutBack(version) ? () => unawaited(_putBack(version)) : null),
      row('menu-copy', l10n.backupOpenCopy, () => unawaited(_copy(version))),
      if (_open)
        row('menu-bring', l10n.backupBringBack,
            () => unawaited(_bring(version))),
      row(
          'menu-name',
          version.note.isEmpty ? l10n.backupNameVersion : l10n.renameEllipsis,
          () => unawaited(_giveName(version))),
      if (version.note.isNotEmpty)
        row('menu-unname', l10n.backupRemoveName,
            () => unawaited(_removeName(version))),
      backupMenuRule(t),
      row('delete', l10n.backupDeleteVersion,
          () => unawaited(_delete(version))),
    ];
  }

  void _showMenu(BackupVersion version, Offset at) {
    final t = ThemeScope.of(context).theme;
    unawaited(showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => _menu(t, version, () => close(null)),
    ));
  }

  /// The one line the window has to say when it has no list to show, with
  /// the way to ask again when the line is that the server was not reached.
  /// Null while there is a list, or one is on its way.
  (String, VoidCallback?)? get _line {
    final versions = _versions;
    if (versions != null && versions.isNotEmpty) return null;
    if (_open && _path == null) return (l10n.backupNotSaved, null);
    if (_error case final error?) {
      return (backupErrorText(error), () => unawaited(_load()));
    }
    if (versions == null && _id != null) return null;
    return (l10n.backupNoVersions, null);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final versions = _versions ?? const <BackupVersion>[];
    final title = dialogTitleBar(
      t,
      title: l10n.backupHistoryTitle,
      subject: _name,
      onClose: widget.close,
      keyPrefix: 'history',
    );
    // Without Pro, and with no history from when there was: what this is,
    // and the way to it. Not before the server has answered, since an
    // account whose Pro has ended still has its history.
    final answered = _versions != null ||
        _error != null ||
        _id == null ||
        !_account.signedIn;
    if (versions.isEmpty && !_account.pro && answered) {
      return DialogFrame(
        width: proWindowWidth,
        children: [
          title,
          ProOffer(
            headline: l10n.backupProHeadline,
            preview: _account.config?.preview ?? false,
          ),
        ],
      );
    }
    final chosen = versions.where((v) => v.n == _chosen).firstOrNull;
    final line = _line;
    return Focus(
      focusNode: _focus,
      autofocus: true,
      onKeyEvent: _key,
      child: DialogFrame(
        // A window with one line to say is as wide as the line wants, and
        // not as wide as the list it has not got.
        width: line == null ? projectHistoryWidth : proWindowWidth,
        children: [
          title,
          // The strip is there when it has something to say: how the open
          // project stands, or what came of the last thing asked.
          if ((_account.pro && _open && _path != null) ||
              _ended ||
              _busy ||
              _said != null)
            _strip(t),
          if (line != null)
            backupLine(t, line.$1, retry: line.$2)
          else if (versions.isEmpty)
            backupWaiting
          else
            _panes(t, versions, chosen),
          if (versions.isNotEmpty)
            dialogFooter(
              t,
              keyPrefix: 'history',
              summary: _count(versions),
              // Bringing one composition back is in words beside the count,
              // and only for the project that is open, which is where it
              // would arrive.
              leading: !_open
                  ? null
                  : Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Flexible(
                          child: Text(_count(versions),
                              key: const ValueKey('history-summary'),
                              style: dialogMono(t),
                              overflow: TextOverflow.ellipsis),
                        ),
                        const SizedBox(width: 14),
                        _bringAction(t, chosen),
                      ],
                    ),
              // The boxes are the decision: look at the version or take it.
              // Naming it is in its row's own menu, with everything else a
              // row has.
              actions: [
                _action('copy', l10n.backupOpenCopy,
                    chosen == null ? null : () => _copy(chosen)),
                // Into the project that is open, so only from its own
                // history.
                if (_open) _putBackButton(t, chosen),
              ],
            ),
        ],
      ),
    );
  }

  /// How many versions there are, and what they come to.
  String _count(List<BackupVersion> versions) => [
        _namedOnly
            ? l10n.backupVersionsShown(
                _listed(versions).length, versions.length)
            : l10n.backupVersions(versions.length),
        if (_project case final project?) backupSize(project.bytes),
      ].join(' · ');

  /// Bring a composition back, in words: the boxes are the two ways to take
  /// the whole version, and this takes one thing out of it.
  Widget _bringAction(LumitTheme t, BackupVersion? chosen) {
    final live = chosen != null && !_busy;
    return HouseButton(
      key: const ValueKey('history-bring'),
      frameless: true,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      onPressed: live ? () => unawaited(_bring(chosen)) : null,
      child: Text(l10n.backupBringBack,
          style: t.small
              .copyWith(color: live ? t.textSecondary : t.textDisabled)),
    );
  }

  Widget _action(String id, String label, VoidCallback? onPressed) =>
      dialogPlainAction(
        ThemeScope.of(context).theme,
        key: ValueKey<String>('history-$id'),
        label: label,
        onPressed: _busy ? null : onPressed,
      );

  /// Put back, the window's one filled button.
  Widget _putBackButton(LumitTheme t, BackupVersion? chosen) {
    final version =
        chosen != null && _canPutBack(chosen) && !_busy ? chosen : null;
    return dialogFilledAction(
      t,
      key: const ValueKey('history-put-back'),
      label: l10n.backupPutBack,
      onPressed: version == null ? null : () => _putBack(version),
    );
  }

  /// Whether Lumit Pro has ended under a history that is still there.
  bool get _ended => !_account.pro && (_versions?.isNotEmpty ?? false);

  /// How the backup stands, the one press that helps when there is one,
  /// and the switch that keeps this project in backup.
  Widget _strip(LumitTheme t) {
    if (_ended && !_busy && _said == null) return backupEndedStrip(context, t);
    final mine = _account.pro && _open && _path != null;
    var (words, tone) =
        mine ? backupStatusWords(_backup) : ('', BackupTone.quiet);
    final off = mine && !_backup.on;
    // A list that has come back empty has nothing backed up in it, whatever
    // this machine last heard: the project was deleted from backup somewhere
    // else. So the strip does not say it is.
    if ((_versions?.isEmpty ?? false) &&
        _backup.status == BackupStatus.backedUp) {
      words = '';
    }
    // A file that is waiting, or was refused, can be sent with a press.
    // Not when the account's storage is full: the press that helps then is
    // the one that leads to where room is made.
    final unsent = mine &&
        _backup.sends &&
        _backup.newer == null &&
        (_backup.status == BackupStatus.waiting ||
            _backup.status == BackupStatus.failed);
    final full = unsent && _backup.error?.code == 'storage_full';
    final newer = mine ? _backup.newer : null;
    final (String, String, VoidCallback)? remedy = _busy
        ? null
        : off
            ? (
                'history-settings',
                l10n.menuSettings,
                () => unawaited(showSettingsWindowFrb(context,
                    initialPage: SettingsPage.account))
              )
            : newer != null
                ? (
                    'history-open-newer',
                    l10n.backupOpenNewer,
                    () => unawaited(_openNewer(newer))
                  )
                : full
                    ? ('history-free-up', l10n.backupFreeUp, _freeUp)
                    : unsent
                        ? (
                            'history-back-up-now',
                            l10n.backupNow,
                            _backup.resend
                          )
                        : null;
    return backupStrip(
      t,
      preview: _account.account?.preview ?? false,
      badge: _account.pro,
      mark: _busy
          ? const SizedBox(width: 12, height: 12, child: SyncRing())
          : _said != null || words.isEmpty || off
              ? null
              : BackupGlyph(_backup),
      words: _busy ? _doing : _said ?? (off ? l10n.backupStatusOff : words),
      tone: _busy
          ? BackupTone.quiet
          : _said != null
              ? BackupTone.refused
              : off
                  ? BackupTone.quiet
                  : tone,
      trailing: [
        if (remedy != null) ...[
          SizedBox(
            height: dialogControlHeight,
            child: HouseButton(
              key: ValueKey<String>(remedy.$1),
              small: true,
              padding: const EdgeInsets.symmetric(horizontal: 10),
              onPressed: remedy.$3,
              child: Text(remedy.$2,
                  style: t.small.copyWith(color: t.textPrimary)),
            ),
          ),
          const SizedBox(width: 16),
        ],
        if (mine) ...[
          Text(l10n.backupThisProject,
              style: t.small.copyWith(color: t.textSecondary)),
          const SizedBox(width: 10),
          // With backup off for every project this switch decides nothing,
          // so it is dimmed and takes no presses, as the one in Settings is
          // without Pro.
          IgnorePointer(
            ignoring: off,
            child: Opacity(
              opacity: off ? 0.4 : 1,
              child: HouseToggle(
                key: const ValueKey('history-keep'),
                value: !_backup.out,
                onChanged: (on) => _backup.setOut(!on),
              ),
            ),
          ),
        ],
      ],
    );
  }

  /// The list on the left and the chosen version on the right.
  Widget _panes(
      LumitTheme t, List<BackupVersion> versions, BackupVersion? chosen) {
    final lastSave = _lastSave(versions);
    return SizedBox(
      height: backupBodyHeight,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(width: _listWidth, child: _list(t, versions, lastSave)),
          Container(width: 1, color: t.hairline),
          Expanded(
            child: chosen == null
                ? const SizedBox.shrink()
                : _detail(t, chosen, chosen.n == lastSave),
          ),
        ],
      ),
    );
  }

  /// The versions, newest first, under the day each was saved on, with the
  /// choice of all of them or only the named ones over them, and under them
  /// what is kept.
  Widget _list(LumitTheme t, List<BackupVersion> all, int? lastSave) {
    final versions = _listed(all);
    final rows = <Widget>[];
    final rowed = <BackupVersion>[];
    final folds = <int>{};
    final runOf = <int, int>{};
    final rowAt = <int, (double, double)>{};
    final dayAt = <(double, String)>[];
    var top = 0.0;
    // Where the run that is open ends. Its rows are not looked at again as
    // the start of a shorter run of their own.
    var open = 0;
    for (var index = 0; index < versions.length; index++) {
      final version = versions[index];
      final first = index == 0;
      if (first || !_sameDay(version.created, versions[index - 1].created)) {
        final day = backupDay(version.created);
        dayAt.add((top, day));
        rows.add(_DayHeading(day, first: first));
        top += _dayHeight;
      }
      // The autosaves from here to the next save, or to the end of the day,
      // are one row until it is opened. The one that is chosen is never
      // folded away.
      if (index >= open) {
        var end = index;
        while (end < versions.length &&
            !_whole(versions[end], all) &&
            _sameDay(versions[end].created, version.created)) {
          end++;
        }
        final run = versions.sublist(index, end);
        if (run.length > 1 &&
            (_foldAt == version.n ||
                (!_unfolded.contains(version.n) &&
                    !run.any((v) => v.n == _chosen)))) {
          rows.add(_FoldRow(
            key: ValueKey<String>('history-fold-${version.n}'),
            count: run.length,
            newest: version.created,
            chosen: _foldAt == version.n,
            above: index > 0,
            below: end < versions.length,
            onOpen: () => setState(() {
              _unfolded.add(version.n);
              _foldAt = null;
            }),
          ));
          // The keys stop here as on any row. It stands for its newest.
          rowAt[version.n] = (top, _quietRowHeight);
          rowed.add(version);
          folds.add(version.n);
          top += _quietRowHeight;
          index = end - 1;
          continue;
        }
        if (run.length > 1) {
          for (final member in run) {
            runOf[member.n] = version.n;
          }
        }
        open = end;
      }
      final whole = _whole(version, all);
      final height = whole ? _rowHeight : _quietRowHeight;
      rowAt[version.n] = (top, height);
      rowed.add(version);
      top += height;
      rows.add(_VersionRow(
        key: ValueKey<int>(version.n),
        backup: _backup,
        id: _id!,
        version: version,
        index: index,
        whole: whole,
        // Said only when it was another machine.
        machine: version.device == AccountState.deviceLabel
            ? ''
            : version.device,
        lastSave: version.n == lastSave,
        chosen: version.n == _chosen,
        last: index == versions.length - 1,
        reveal: _fresh && index < 8,
        onChoose: () => _choose(version),
        onMenu: (at) => _showMenu(version, at),
      ));
    }
    _rowed = rowed;
    _folds = folds;
    _runOf = runOf;
    _rowAt = rowAt;
    _dayAt = dayAt;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          height: _listHeadHeight,
          padding: const EdgeInsets.only(left: _railAt * 2, right: 14),
          alignment: Alignment.centerLeft,
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: t.hairline)),
          ),
          // Two chips side by side with the one in force outlined, as
          // Settings draws a choice of two.
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final (named, label) in [
                (false, l10n.backupFilterAll),
                (true, l10n.backupFilterNamed),
              ]) ...[
                if (named) const SizedBox(width: 2),
                SizedBox(
                  height: dialogControlHeight,
                  child: HouseButton(
                    key: ValueKey<String>(
                        named ? 'history-show-named' : 'history-show-all'),
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    frameless: _namedOnly != named,
                    onPressed: () => _showNamedOnly(named),
                    child: Text(t.kickerCase(label),
                        style: _namedOnly == named ? t.kickerOn : t.kicker),
                  ),
                ),
              ],
            ],
          ),
        ),
        // Where the project is, when that is ahead of everything listed.
        if (_ahead(all))
          Container(
            key: const ValueKey('history-ahead'),
            height: _quietRowHeight,
            decoration: BoxDecoration(
              border: Border(bottom: BorderSide(color: t.hairline)),
            ),
            child: Row(
              children: [
                const _Rail(
                    height: _quietRowHeight,
                    dot: true,
                    lastSave: true,
                    above: false,
                    below: false),
                Expanded(
                  child: Text(l10n.backupAhead,
                      style: t.small.copyWith(color: t.textPrimary),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis),
                ),
                const SizedBox(width: 14),
              ],
            ),
          ),
        Expanded(
          child: Stack(
            clipBehavior: Clip.hardEdge,
            children: [
              NotificationListener<ScrollUpdateNotification>(
                onNotification: (_) {
                  _fresh = false;
                  return false;
                },
                // The scrollbar Settings has: a long history is travelled
                // by taking hold of it. It starts under the day held at
                // the top.
                child: dialogGutter(
                  context,
                  t,
                  _scroll,
                  top: rows.isEmpty ? 0 : _dayHeight,
                  child: ListView(
                    key: const ValueKey('history-list'),
                    controller: _scroll,
                    padding: const EdgeInsets.only(bottom: 8),
                    children: [
                      if (rows.isEmpty)
                        Padding(
                          padding:
                              const EdgeInsets.fromLTRB(_railAt * 2, 14, 14, 6),
                          child: Text(l10n.backupNoNamed,
                              style: t.body.copyWith(color: t.textSecondary)),
                        ),
                      ...rows,
                    ],
                  ),
                ),
              ),
              if (dayAt.isNotEmpty) _heldDay(t),
            ],
          ),
        ),
        // What is kept, in one line held under the list, whichever versions
        // it is showing and however far it has been scrolled. The rule is
        // there for whoever stops on it.
        Container(
          padding: const EdgeInsets.fromLTRB(_railAt * 2, 8, 14, 9),
          decoration: BoxDecoration(
            border: Border(top: BorderSide(color: t.hairline)),
          ),
          child: LumitTooltip(
            message: l10n.backupKeptRule,
            maxWidth: 320,
            child: Text(l10n.backupKept,
                key: const ValueKey('history-kept'),
                style: t.small,
                maxLines: 1,
                overflow: TextOverflow.ellipsis),
          ),
        ),
      ],
    );
  }

  /// The day of the topmost row in view, held at the top of the list, so a
  /// time in it is never without its date. The next day's heading pushes
  /// this one up and out as it arrives.
  Widget _heldDay(LumitTheme t) => ListenableBuilder(
        listenable: _scroll,
        builder: (context, _) {
          final at = _scroll.hasClients ? _scroll.offset : 0.0;
          var day = _dayAt.first.$2;
          var push = 0.0;
          for (final (top, name) in _dayAt.skip(1)) {
            if (top > at) {
              push = math.min(0.0, top - at - _dayHeight);
              break;
            }
            day = name;
          }
          // A pixel over the top, which the list's own edge cuts off: at an
          // interface scale that lands that edge between two pixels, a row
          // passing under would otherwise show through the half one.
          return Positioned(
            top: push - 1,
            left: 0,
            right: 0,
            height: _dayHeight + 1,
            child: Container(
              padding: const EdgeInsets.only(left: _railAt * 2, top: 1),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: t.surface1,
                // Ruled off once rows are passing under it.
                border: Border(
                  bottom: BorderSide(
                      color: t.hairline.withValues(alpha: at > 0 ? null : 0)),
                ),
              ),
              child: Text(t.kickerCase(day),
                  key: const ValueKey('history-day'), style: t.kicker),
            ),
          );
        },
      );

  /// The chosen version: its picture, which stays put, over what is known
  /// about it and what changed in it, which scroll when they are long.
  Widget _detail(LumitTheme t, BackupVersion version, bool lastSave) {
    final changed = _whatChanged[version.n] ?? const [];
    return Padding(
      padding: const EdgeInsets.only(top: _detailInset),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: _detailInset),
            child: BackupStill(_backup, _id!, version, whole: true),
          ),
          // The picture's own line stays put under it, so words passing
          // under never touch it, and they are ruled off once they move.
          ListenableBuilder(
            listenable: _detailScroll,
            builder: (context, _) => Container(
              height: 1,
              margin: const EdgeInsets.symmetric(horizontal: _detailInset),
              color: t.hairline.withValues(
                  alpha: _detailScroll.hasClients && _detailScroll.offset > 0
                      ? null
                      : 0),
            ),
          ),
          Expanded(
            // Where there are more words than fit, the last of them fade
            // out and are not cut through by the foot.
            child: Stack(
              fit: StackFit.expand,
              children: [
                _detailWords(t, version, lastSave, changed),
                Positioned(
                  left: 0,
                  right: 8,
                  bottom: 0,
                  height: 24,
                  child: IgnorePointer(
                    child: ValueListenableBuilder<bool>(
                      valueListenable: _moreWords,
                      builder: (context, more, _) => more
                          ? DecoratedBox(
                              decoration: BoxDecoration(
                                gradient: LinearGradient(
                                  begin: Alignment.topCenter,
                                  end: Alignment.bottomCenter,
                                  colors: [
                                    t.surface1.withValues(alpha: 0),
                                    t.surface1,
                                  ],
                                ),
                              ),
                            )
                          : const SizedBox.shrink(),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// What is known about the chosen version and what changed in it.
  Widget _detailWords(LumitTheme t, BackupVersion version, bool lastSave,
      List<(int, String)> changed) {
    final most = changed.fold(0, (most, c) => math.max(most, c.$1));
    final countWidth = math.max(
        20.0, monoSlotWidth(t.mono.copyWith(fontSize: 11), '$most'.length));
    return NotificationListener<Notification>(
      // Whether there are words out of sight below, as the list of what
      // changed arrives and as it is scrolled.
      onNotification: (said) {
        final metrics = switch (said) {
          ScrollMetricsNotification(:final metrics) => metrics,
          ScrollUpdateNotification(:final metrics) => metrics,
          _ => null,
        };
        if (metrics != null) _moreWords.value = metrics.extentAfter > 1;
        return false;
      },
      child: dialogScroller(
        context,
        t,
        _detailScroll,
        child: SingleChildScrollView(
          controller: _detailScroll,
          padding: const EdgeInsets.fromLTRB(
              _detailInset, 7, _detailInset, _detailInset),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Text(t.kickerCase(l10n.backupVersionNumber(version.n)),
                      style: t.kicker),
                  if (version.auto) ...[
                    const SizedBox(width: 10),
                    Text(t.kickerCase(l10n.backupAutosave),
                        style: t.kicker),
                  ],
                  if (lastSave) ...[
                    const SizedBox(width: 10),
                    const _LastSaveTag(),
                  ],
                ],
              ),
              const SizedBox(height: 8),
              Text(
                version.note.isEmpty
                    ? backupWhen(version.created)
                    : version.note,
                key: const ValueKey('history-chosen'),
                style: t.heading,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
              if (version.note.isNotEmpty) ...[
                const SizedBox(height: 4),
                Text(backupWhen(version.created),
                    style: t.body.copyWith(color: t.textSecondary)),
              ],
              const SizedBox(height: 8),
              Text(
                [
                  if (version.device.isNotEmpty) version.device,
                  backupSize(version.size),
                  if (version.app.isNotEmpty) 'Lumit ${version.app}',
                ].join(' · '),
                style: dialogMono(t)
                    .copyWith(fontWeight: FontWeight.w400),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
              if (changed.isNotEmpty) ...[
                const SizedBox(height: 12),
                Text(t.kickerCase(l10n.backupChanged), style: t.kicker),
                const SizedBox(height: 2),
                // One change to a line: how many, then what. The counts
                // share one column, as wide as the biggest of them.
                Column(
                  key: const ValueKey('history-changes'),
                  children: [
                    for (final (count, words) in changed)
                      Padding(
                        padding: const EdgeInsets.only(top: 4),
                        child: Row(
                          crossAxisAlignment:
                              CrossAxisAlignment.baseline,
                          textBaseline: TextBaseline.alphabetic,
                          children: [
                            SizedBox(
                              width: countWidth,
                              child: Text('$count',
                                  textAlign: TextAlign.right,
                                  maxLines: 1,
                                  softWrap: false,
                                  style: t.mono.copyWith(fontSize: 11)),
                            ),
                            const SizedBox(width: 8),
                            Expanded(child: Text(words, style: t.body)),
                          ],
                        ),
                      ),
                  ],
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// The hairline between two runs of rows in a menu.
Widget backupMenuRule(LumitTheme t) => Container(
      height: 1,
      margin: const EdgeInsets.symmetric(vertical: 4),
      color: t.hairline,
    );

/// The words that mark the version the open project's file is.
class _LastSaveTag extends StatelessWidget {
  const _LastSaveTag();

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Text(t.kickerCase(l10n.backupLastSave), style: t.kickerOn);
  }
}

/// The rail down the side of the list, with a mark where a version sits on
/// it: a small dot for an autosave, a ring for a save, and the diamond of a
/// value that is set for a version with a name. The mark of the version the
/// project's file is, is filled.
class _Rail extends StatelessWidget {
  final double height;
  final bool dot, lastSave, small, named, above, below;

  const _Rail(
      {required this.height,
      this.dot = false,
      this.lastSave = false,
      this.small = false,
      this.named = false,
      this.above = true,
      this.below = true});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final middle = height / 2;
    final across = small ? _dot - 3 : _dot;
    // The line stops short of a mark, so a hollow one stays hollow.
    final gap = dot ? across / 2 + 3 : 0.0;
    Widget line(double top, double bottom) => Positioned(
          left: _railAt,
          top: top,
          bottom: bottom,
          child: Container(width: 1, color: t.hairline),
        );
    // The key's own diamond is drawn three quarters of the way across its
    // box, so the box is a third larger than the mark is to be.
    const diamond = _dot * 4 / 3;
    return SizedBox(
      width: _railAt * 2,
      height: height,
      child: Stack(
        children: [
          if (above) line(0, middle + gap),
          if (below) line(middle + gap, 0),
          if (dot && named)
            Positioned(
              left: _railAt + 0.5 - diamond / 2,
              top: middle - diamond / 2,
              child: glyph.LumitIcon(LumitIcons.selectedKey,
                  size: diamond,
                  colour: lastSave ? t.textPrimary : t.animated),
            )
          else if (dot)
            Positioned(
              left: _railAt + 0.5 - across / 2,
              top: middle - across / 2,
              child: Container(
                width: across,
                height: across,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: lastSave
                      ? t.textPrimary
                      : small
                          ? t.hairlineStrong
                          : null,
                  border: lastSave || small
                      ? null
                      : Border.all(color: t.textMuted, width: 1.2),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// A run of autosaves between two saves, as one quiet line that says how
/// many there are and when the newest was. A press opens it in place.
class _FoldRow extends StatefulWidget {
  final int count;
  final DateTime newest;

  /// The keys are on it.
  final bool chosen;
  final bool above, below;
  final VoidCallback onOpen;

  const _FoldRow(
      {super.key,
      required this.count,
      required this.newest,
      required this.chosen,
      required this.above,
      required this.below,
      required this.onOpen});

  @override
  State<_FoldRow> createState() => _FoldRowState();
}

class _FoldRowState extends State<_FoldRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onOpen,
        child: Container(
          height: _quietRowHeight,
          color: widget.chosen
              ? t.selectionFill
              : _hover
                  ? t.surface2
                  : null,
          foregroundDecoration: widget.chosen
              ? BoxDecoration(
                  border: Border(left: BorderSide(color: t.accent, width: 2)),
                )
              : null,
          child: Row(
            children: [
              _Rail(
                height: _quietRowHeight,
                dot: true,
                small: true,
                above: widget.above,
                below: widget.below,
              ),
              SizedBox(
                width: 40,
                child: Text(
                  backupTime(widget.newest),
                  style: t.mono.copyWith(
                      fontSize: 10,
                      color: widget.chosen ? t.textPrimary : t.textMuted),
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  l10n.backupAutosaves(widget.count),
                  style: t.small.copyWith(
                      color:
                          _hover || widget.chosen ? t.textSecondary : null),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              // The mark a closed group has everywhere else in Lumit.
              SizedBox(
                width: 30,
                child: glyph.LumitIcon(LumitIcons.expand,
                    size: 12, colour: _hover ? t.textSecondary : t.textMuted),
              ),
              const SizedBox(width: 6),
            ],
          ),
        ),
      ),
    );
  }
}

class _DayHeading extends StatelessWidget {
  final String day;
  final bool first;
  const _DayHeading(this.day, {required this.first});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      height: _dayHeight,
      child: Row(
        children: [
          // Nothing comes before the newest day, so the rail starts under it.
          first
              ? const SizedBox(width: _railAt * 2)
              : const _Rail(height: _dayHeight),
          Text(t.kickerCase(day), style: t.kicker),
        ],
      ),
    );
  }
}

/// One version in the list: when it was saved, how it looked, and what it
/// is, which is its name, a save or an autosave. An autosave nobody has
/// named is one quiet line with no picture.
class _VersionRow extends StatefulWidget {
  final BackupState backup;
  final String id;
  final BackupVersion version;
  final int index;

  /// Whether it gets a whole row, with its picture.
  final bool whole;

  /// The machine it was saved on when that was not this one, and empty when
  /// it was.
  final String machine;
  final bool lastSave, chosen, last, reveal;
  final VoidCallback onChoose;

  /// Asks for the row's menu, at a place on the screen.
  final void Function(Offset at) onMenu;

  const _VersionRow({
    super.key,
    required this.backup,
    required this.id,
    required this.version,
    required this.index,
    required this.whole,
    required this.machine,
    required this.lastSave,
    required this.chosen,
    required this.last,
    required this.reveal,
    required this.onChoose,
    required this.onMenu,
  });

  @override
  State<_VersionRow> createState() => _VersionRowState();
}

class _VersionRowState extends State<_VersionRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final version = widget.version;
    final named = version.note.isNotEmpty;
    final whole = widget.whole;
    final height = whole ? _rowHeight : _quietRowHeight;
    final what = version.auto ? l10n.backupAutosave : l10n.backupSaveRow;
    // Under its name or what it is, only what that does not say already.
    final under = [
      if (named && version.auto) l10n.backupAutosave,
      if (widget.machine.isNotEmpty) widget.machine,
    ].join(' · ');
    final row = MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: ValueKey<String>('history-version-${version.n}'),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onChoose,
        // A right click is about the row it lands on.
        onSecondaryTapDown: (press) {
          widget.onChoose();
          widget.onMenu(press.globalPosition);
        },
        child: Container(
          height: height,
          color: widget.chosen
              ? t.selectionFill
              : _hover
                  ? t.surface2
                  : null,
          // The chosen row wears the edge the page in force wears in
          // Settings. Painted over the row, so nothing in it moves.
          foregroundDecoration: widget.chosen
              ? BoxDecoration(
                  border: Border(left: BorderSide(color: t.accent, width: 2)),
                )
              : null,
          child: Row(
            children: [
              _Rail(
                height: height,
                dot: true,
                lastSave: widget.lastSave,
                small: version.auto && !named,
                named: named,
                above: widget.index > 0,
                below: !widget.last,
              ),
              SizedBox(
                width: 40,
                child: Text(
                  backupTime(version.created),
                  style: t.mono.copyWith(
                      fontSize: 10,
                      color: widget.chosen || widget.lastSave
                          ? t.textPrimary
                          : whole
                              ? t.textSecondary
                              : t.textMuted),
                ),
              ),
              const SizedBox(width: 8),
              if (!whole)
                Expanded(
                  child: Text(
                    what,
                    style: t.small.copyWith(
                        color: widget.chosen ? t.textSecondary : t.textMuted),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                )
              else ...[
                BackupStill(widget.backup, widget.id, version,
                    width: 64, height: 36),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Row(
                        children: [
                          Flexible(
                            child: named
                                ? FittedText(version.note, t.bodyPrimary)
                                : Text(what, style: t.body),
                          ),
                          if (widget.lastSave) ...[
                            const SizedBox(width: 8),
                            const _LastSaveTag(),
                          ],
                        ],
                      ),
                      if (under.isNotEmpty) ...[
                        const SizedBox(height: 3),
                        Text(
                          under,
                          style: t.small,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ],
                    ],
                  ),
                ),
              ],
              SizedBox(
                width: 30,
                child: _hover || widget.chosen
                    ? Builder(
                        builder: (button) => HouseButton(
                          key: ValueKey<String>('history-more-${version.n}'),
                          frameless: true,
                          small: true,
                          padding: EdgeInsets.zero,
                          onPressed: () {
                            final box = button.findRenderObject();
                            if (box is! RenderBox) return;
                            widget.onMenu(
                                box.localToGlobal(Offset(0, box.size.height)));
                          },
                          // The chosen row's is the way to everything the
                          // foot of the window does not offer.
                          child: glyph.LumitIcon(LumitIcons.more,
                              size: 14,
                              colour: widget.chosen
                                  ? t.textSecondary
                                  : t.textMuted),
                        ),
                      )
                    : null,
              ),
              const SizedBox(width: 6),
            ],
          ),
        ),
      ),
    );
    final motion = scope.motion;
    // A shape whose reveals don't rise has nothing to stagger.
    if (!widget.reveal || !motion.carries || motion.revealRise == 0) {
      return row;
    }
    // Held back a beat for each row above it, then up and in, the way the
    // Lumit Pro window brings in what Pro does.
    final wait = widget.index * 45;
    return TweenAnimationBuilder<double>(
      tween: Tween(begin: 0, end: 1),
      duration: Duration(milliseconds: 240 + wait),
      curve: Interval(wait / (240 + wait), 1, curve: motion.reveal.curve),
      builder: (context, shown, child) => Opacity(
        opacity: shown,
        child: Transform.translate(
            offset: Offset(0, (1 - shown) * motion.revealRise), child: child),
      ),
      child: row,
    );
  }
}
