// The Open from backup window: the projects an account has in backup, and
// the way to bring one onto this machine.
//
// Each project is a card: how it last looked, what it is called, when it
// was last saved, on which machine when that was another one, and how many
// versions are kept. A project whose file is already on this machine opens
// that file. One that is not is fetched to wherever the person says, and
// opened from there.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/account.dart';
import '../state/backup.dart';
import '../state/file_dialogs.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'pro_window.dart';
import 'profile_chip.dart';
import 'project_history_window.dart';
import 'settings_account.dart' show askBefore;

const double backupsWidth = 800;

/// The grid: three cards across, and the air round and between them. The
/// air round them is the inset every other edge of the window takes.
const int _across = 3;
const double _edge = dialogPadding;
const double _gap = 14;

/// A card's own inset, and the room under its picture for its three lines.
const double _inset = 8;
const double _words = 72;

/// How wide a card is, and how tall.
const double _card =
    (backupsWidth - _edge * 2 - _gap * (_across - 1)) / _across;
const double _cardHeight = _inset + (_card - _inset * 2) * 9 / 16 + _words;

/// Where [project]'s file is on this machine. When it has none here its
/// newest version is fetched to wherever the person says, starting in the
/// folder the last one went to. Null when they backed out.
///
/// [picker] takes the name to offer and the folder to start in, for a test,
/// which can't open a real dialogue.
Future<String?> bringBackedUp(LumitUiState ui, BackupProject project,
    {Future<String?> Function(String name, String folder)? picker}) async {
  final backup = ui.backup;
  final here = backup.localPath(project.id);
  final head = project.head;
  if (here != null || head == null) return here;
  final picked = await (picker ??
      (name, folder) => pickProjectSaveLocation(
          suggestedName: name, initialDirectory: folder))(
      '${project.name}.lum', backup.folder);
  if (picked == null) return null;
  // The Windows dialogue hands back a typed name as it is.
  final path = picked.toLowerCase().endsWith('.lum') ? picked : '$picked.lum';
  await backup.fetch(project.id, head, path, home: true);
  return path;
}

/// Open the Open from backup window. [full] says it was a save refused for
/// want of storage that led here, so the line that says how much is used is
/// the thing to read.
Future<void> showBackups(BuildContext context, {bool full = false}) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  return showLumitModal<void>(
    context: context,
    id: 'backups',
    builder: (close) =>
        _Backups(app: app, ui: ui, full: full, close: () => close(null)),
  );
}

class _Backups extends StatefulWidget {
  final LumitState app;
  final LumitUiState ui;
  final bool full;
  final VoidCallback close;

  const _Backups(
      {required this.app,
      required this.ui,
      required this.full,
      required this.close});

  @override
  State<_Backups> createState() => _BackupsState();
}

class _BackupsState extends State<_Backups> {
  final _search = TextEditingController();
  final _scroll = ScrollController();

  /// The project being fetched, and what the last thing asked for came back
  /// with when it failed.
  String? _fetching;
  String? _said;

  BackupState get _backup => widget.ui.backup;
  AccountState get _account => widget.ui.account;

  /// Whether the cards were in hand as the window opened. Then they arrive
  /// with the window, and otherwise they come up into place when the server
  /// answers.
  late final bool _listedAtOpen;

  @override
  void initState() {
    super.initState();
    _listedAtOpen = _backup.projects.isNotEmpty;
    _backup.addListener(_changed);
    _account.addListener(_changed);
    _search.addListener(_changed);
    if (_account.signedIn) unawaited(_backup.refresh());
  }

  @override
  void dispose() {
    _backup.removeListener(_changed);
    _account.removeListener(_changed);
    _search.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  /// Run something the person asked for, and say its refusal in the strip.
  Future<void> _run(Future<void> Function() what) async {
    setState(() => _said = null);
    try {
      await what();
    } on CloudError catch (e) {
      if (mounted) setState(() => _said = backupErrorText(e));
    }
  }

  Future<void> _open(BackupProject project) async {
    if (_fetching != null) return;
    setState(() => _fetching = project.id);
    final (app, ui) = (widget.app, widget.ui);
    await _run(() async {
      final path = await bringBackedUp(ui, project);
      if (path == null) return;
      widget.close();
      await app.openProject(path);
      await noticeMissingFootage(app, ui);
    });
    if (mounted) setState(() => _fetching = null);
  }

  Future<void> _rename(BackupProject project) async {
    final name = await askProfileName(context,
        title: l10n.backupRenameTitle,
        suggested: project.name,
        confirm: l10n.rename);
    if (name != null) await _run(() => _backup.rename(project.id, name));
  }

  Future<void> _delete(BackupProject project) async {
    final sure = await askBefore(
        context, l10n.backupDeleteAsk(project.name), l10n.delete);
    if (sure) await _run(() => _backup.delete(project.id));
  }

  /// Everything there is to do with [project], for its card's own menu and
  /// for a right click. Open comes first: it is what the window is for.
  List<Widget> _menu(LumitTheme t, BackupProject project, VoidCallback close) {
    MenuRow row(String id, String label, VoidCallback then) => MenuRow(
          key: ValueKey<String>('backups-$id-${project.id}'),
          onPressed: () {
            close();
            then();
          },
          child: Text(label),
        );
    return [
      row('open', l10n.backupCardOpen, () => unawaited(_open(project))),
      row(
          'history',
          l10n.backupMenuHistory,
          () => unawaited(showProjectHistory(context,
              of: project.id == _backup.id ? null : project))),
      row('rename', l10n.renameEllipsis, () => unawaited(_rename(project))),
      backupMenuRule(t),
      row('delete', l10n.backupDelete, () => unawaited(_delete(project))),
    ];
  }

  void _showMenu(BackupProject project, Offset at) {
    final t = ThemeScope.of(context).theme;
    unawaited(showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => _menu(t, project, () => close(null)),
    ));
  }

  /// The one line the window has to say when it has no cards to show, with
  /// the way to ask again when the line is that the server was not reached.
  /// Null while there are cards, or the list is on its way.
  (String, VoidCallback?)? _line(List<BackupProject> all) {
    if (all.isNotEmpty) return null;
    if (_backup.listError case final error?) {
      return (backupErrorText(error), () => unawaited(_backup.refresh()));
    }
    return _backup.listed ? (l10n.backupNothing, null) : null;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final all = _backup.projects;
    final title = dialogTitleBar(
      t,
      title: l10n.backupOpenTitle,
      onClose: widget.close,
      keyPrefix: 'backups',
    );
    // Without Pro, and with nothing in backup from when there was: what
    // this is, and the way to it. Not before the server has answered, since
    // an account whose Pro has ended still has its projects.
    if (all.isEmpty && !_account.pro) {
      final answered = _backup.listed ||
          _backup.listError != null ||
          !_account.signedIn;
      return DialogFrame(
        width: proWindowWidth,
        children: [
          title,
          if (answered)
            ProOffer(
              headline: l10n.backupProHeadline,
              preview: _account.config?.preview ?? false,
            )
          else
            backupWaitingLine,
        ],
      );
    }
    final line = _line(all);
    return DialogFrame(
      // A window with one line to say is as wide as the line wants.
      width: line == null ? backupsWidth : proWindowWidth,
      children: [
        title,
        // Pro has ended and the projects are still there: the strip says so,
        // and the search box stays.
        if (!_account.pro && _said == null)
          backupEndedStrip(context, t, trailing: [
            if (all.length > 8) ...[const SizedBox(width: 16), _searchBox(t)],
          ])
        else
          backupStrip(
            t,
            preview: _account.account?.preview ?? false,
            badge: _account.pro,
            words: _said ??
                (_backup.limit > 0
                    ? l10n.backupStorageIn(
                        backupSize(_backup.used),
                        backupSize(_backup.limit),
                        backupSize(_backup.stored))
                    : ''),
            tone: _said != null || widget.full
                ? BackupTone.refused
                : BackupTone.quiet,
            trailing: [if (all.length > 8) _searchBox(t)],
          ),
        if (line != null)
          backupLine(t, line.$1, retry: line.$2)
        else if (all.isEmpty)
          backupWaiting
        else ...[
          Entrance(
            spec: _listedAtOpen
                ? MotionSpec.still
                : ThemeScope.of(context).motion.reveal,
            rise: ThemeScope.of(context).motion.revealRise,
            child: _grid(t, all),
          ),
          // Said once, where a project is about to be relied on.
          Container(
            padding: const EdgeInsets.fromLTRB(_edge, 9, _edge, 10),
            decoration: BoxDecoration(
              border: Border(top: BorderSide(color: t.hairline)),
            ),
            child: Text(l10n.backupFootageNote,
                key: const ValueKey('backups-footage'), style: t.small),
          ),
        ],
      ],
    );
  }

  Widget _searchBox(LumitTheme t) => SizedBox(
        height: dialogControlHeight,
        child: HouseTextField(
          key: const ValueKey('backups-search'),
          controller: _search,
          width: 220,
          hint: l10n.backupSearch,
          leading:
              glyph.LumitIcon(LumitIcons.search, size: 12, colour: t.textMuted),
        ),
      );

  /// The cards. The window shows two whole rows of them, or one when one is
  /// all there are, and the thumb down its side says when there are more.
  Widget _grid(LumitTheme t, List<BackupProject> all) {
    final wanted = _search.text.trim().toLowerCase();
    final shown = [
      for (final project in all)
        if (project.name.toLowerCase().contains(wanted)) project,
    ];
    final rows = all.length > _across ? 2 : 1;
    return SizedBox(
      height: _edge * 2 + _cardHeight * rows + _gap * (rows - 1),
      child: shown.isEmpty
          ? Center(
              child: Text(l10n.noMatches,
                  style: t.body.copyWith(color: t.textSecondary)),
            )
          : dialogScroller(
              context,
              t,
              _scroll,
              child: GridView(
                key: const ValueKey('backups-grid'),
                controller: _scroll,
                padding: const EdgeInsets.all(_edge),
                gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
                  crossAxisCount: _across,
                  mainAxisSpacing: _gap,
                  crossAxisSpacing: _gap,
                  mainAxisExtent: _cardHeight,
                ),
                children: [
                  for (final project in shown)
                    _Card(
                      key: ValueKey<String>('backups-card-${project.id}'),
                      backup: _backup,
                      project: project,
                      fetching: _fetching == project.id,
                      onOpen: () => unawaited(_open(project)),
                      onMenu: (at) => _showMenu(project, at),
                    ),
                ],
              ),
            ),
    );
  }
}

/// One backed-up project, and the whole of it is the button.
class _Card extends StatefulWidget {
  final BackupState backup;
  final BackupProject project;
  final bool fetching;
  final VoidCallback onOpen;

  /// Asks for the card's menu, at a place on the screen.
  final void Function(Offset at) onMenu;

  const _Card({
    super.key,
    required this.backup,
    required this.project,
    required this.fetching,
    required this.onOpen,
    required this.onMenu,
  });

  @override
  State<_Card> createState() => _CardState();
}

class _CardState extends State<_Card> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final project = widget.project;
    final head = project.head;
    final step = _hover ? scope.motion.hoverIn : scope.motion.hoverOut;
    return GestureDetector(
      onSecondaryTapDown: (press) => widget.onMenu(press.globalPosition),
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onOpen,
          child: AnimatedContainer(
            duration: step.duration,
            curve: step.curve,
            padding: const EdgeInsets.all(_inset),
            decoration: BoxDecoration(
              color: _hover ? t.surface2 : t.surface1,
              borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
              border:
                  Border.all(color: _hover ? t.hairlineStrong : t.hairline),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                AspectRatio(
                  aspectRatio: 16 / 9,
                  child: Stack(
                    fit: StackFit.expand,
                    children: [
                      BackupStill(widget.backup, project.id, head),
                      if (widget.fetching)
                        Container(
                          color: t.surface0.withValues(alpha: 0.72),
                          alignment: Alignment.center,
                          child: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              const SizedBox(
                                  width: 14, height: 14, child: SyncRing()),
                              const SizedBox(width: 8),
                              Text(l10n.backupFetching,
                                  style: t.small
                                      .copyWith(color: t.textSecondary)),
                            ],
                          ),
                        ),
                    ],
                  ),
                ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(4, 10, 0, 0),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: FittedText(project.name,
                                t.bodyPrimary.copyWith(fontSize: 12)),
                          ),
                          Builder(
                            builder: (button) => HouseButton(
                              key: ValueKey<String>(
                                  'backups-more-${project.id}'),
                              frameless: true,
                              small: true,
                              padding: EdgeInsets.zero,
                              onPressed: () {
                                final box = button.findRenderObject();
                                if (box is! RenderBox) return;
                                widget.onMenu(box.localToGlobal(
                                    Offset(0, box.size.height)));
                              },
                              child: glyph.LumitIcon(LumitIcons.more,
                                  size: 14,
                                  colour:
                                      _hover ? t.textSecondary : t.textMuted),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 3),
                      Text(
                        [
                          backupWhen(project.updated),
                          // Said only when it was another machine.
                          if (head != null &&
                              head.device.isNotEmpty &&
                              head.device != AccountState.deviceLabel)
                            head.device,
                        ].join(' · '),
                        style: t.small.copyWith(color: t.textSecondary),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                      const SizedBox(height: 4),
                      // How many versions, and what they come to: the
                      // figure to go by when room has to be made.
                      Text(
                          [
                            l10n.backupVersions(project.versions),
                            backupSize(project.bytes),
                          ].join(' · '),
                          style: t.small.copyWith(color: t.textMuted)),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
