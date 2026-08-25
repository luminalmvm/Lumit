// The welcome screen — the page Lumit opens on (docs/07-UI-SPEC.md §13.2,
// K-448, K-464).
//
// In plain terms: before any project is on screen, Lumit shows one calm page.
// The wordmark, three ways to start work, the projects worked on recently, and
// two links out to the manual and the release notes. Picking any of them hands
// the window over to the editor, and the page is not shown again until the next
// launch.
//
// **It renders in this window**, not in one of its own. A separate welcome
// window is where docs/impl/multi-window.md ends up; Flutter's windowing is not
// shippable yet (K-449), so the page takes the main window after the boot
// splash and gives it up to the shell — the same handover the splash makes.
//
// Nothing here decides anything. Every card runs the very function its File
// menu row runs (`openProjectFrb`, `saveProjectFrb`, `LumitState.newProject`),
// so there is one implementation of "open a project" and the welcome is a
// second way to reach it rather than a second copy of it.
//
// The same three cards are also what the Viewer shows when the welcome has been
// closed and nothing is open ([EmptyStageFrb], K-481) — the one list of ways to
// start work, in the two places somebody can be standing when they have not
// started yet.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/placeholder.dart';
import '../state/external_links.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'about_window_frb.dart';
import 'menu_bar_frb.dart';
import 'wordmark.dart';

// --- The drawing's measurements -------------------------------------------
//
// Every one of these is read off the approved welcome mockup (K-451: the
// mockup's metrics are canonical, and a mismatch is a defect). They are
// declared here rather than written into the widgets so `welcome_metrics_test`
// can assert against the same numbers the screen is built from.

/// The width of the whole stack — cards, recents and footer alike.
const double welcomeColumnWidth = 560;

/// The air between the four blocks: wordmark, cards, recents, footer.
const double welcomeBlockGap = 28;

/// The wordmark: the brand's own lockup rather than the word set in type
/// (K-480), 22 logical pixels from its cap line to the `u`'s overshoot — the
/// height the drawing's 22px mono mark stood at. Its width follows the lockup.
const double welcomeWordmarkHeight = 22;

/// A start card: 14 of padding above and below a 13px title, a 4px gap and a
/// 9px note, with the hairline counted in.
const double welcomeCardHeight = 63;
const double welcomeCardGap = 10;
const EdgeInsets welcomeCardPadding = EdgeInsets.symmetric(
  horizontal: 16,
  vertical: 14,
);

/// The kicker strip over the recents well — the word and 6px under it.
const double welcomeRecentHeaderHeight = 18;

/// One recent project. The seam under it is drawn on top, so a row measures 52
/// and the eye reads 53 for all but the last.
///
/// 52 rather than the drawing's 40 because the row now carries a picture
/// (K-468): 8 of air above and below a 36-tall thumbnail is what sets it.
const double welcomeRecentRowHeight = 52;
const EdgeInsets welcomeRecentRowPadding = EdgeInsets.symmetric(horizontal: 14);

/// The thumbnail at the head of a row: the project as it looked when it was
/// last saved. 16:9 exactly, and sized to the row — 36 tall leaves 8 either
/// side of it, and 64 is the width that ratio asks for.
const double welcomeThumbHeight = 36;
const double welcomeThumbWidth = 64;

/// The air between the thumbnail and the name column.
const double welcomeThumbGap = 12;

/// The recents' fixed columns: the date, and the button that forgets the row.
/// The name column takes whatever is left.
///
/// **There is no format column** (K-468). It was drawn holding a project's size
/// and rate and reserved 120px against an engine call that could report them
/// without opening the file — but that is per-composition data and a project
/// has as many of those as it likes, so the question the column asked was the
/// wrong one. Its room went to the picture and to the name.
const double welcomeDateColumnWidth = 70;
const double welcomeForgetColumnWidth = 12;

/// The air after the name column. The format column used to hold the name and
/// the date apart; with it gone this is what keeps an elided path off the date.
const double welcomeNameGap = 16;

/// The gap between the date and the forget button. The drawing has no forget
/// button — it is the owner's addition on top of it — so the name column is
/// what gives way to make room, which is step 1 of §12A.6's ladder.
const double welcomeForgetGap = 8;

/// The footer strip, and the outlined buttons in it. 22 of content inside a
/// hairline either side.
const double welcomeFooterHeight = 28;
const double welcomeButtonHeight = 24;

/// The note under a card's title, and the two sentence-case kickers beside the
/// recents heading: the kicker face at the drawing's looser 0.06em rather than
/// the 0.12em a container label carries.
TextStyle welcomeNote(LumitTheme t) => t.kicker.copyWith(letterSpacing: 0.54);

/// A recent project's factual column — when it was last opened — mono at 10,
/// muted, exactly as the drawing sets it.
TextStyle welcomeFact(LumitTheme t) =>
    t.mono.copyWith(fontSize: 10, color: t.textMuted);

/// The screen itself.
///
/// [onDone] takes the window down and puts the shell up. Everything that opens
/// a document calls it; the two links and the recents' own housekeeping do not,
/// because reading the manual is not starting work.
class WelcomeScreenFrb extends StatefulWidget {
  final VoidCallback onDone;

  /// The file pickers, so a widget test can answer them — no plugin channel
  /// opens a real dialogue in one. The same seam `openProjectFrb` and
  /// `saveProjectFrb` already take.
  final Future<String?> Function()? openPicker;
  final Future<String?> Function()? savePicker;

  const WelcomeScreenFrb({
    super.key,
    required this.onDone,
    this.openPicker,
    this.savePicker,
  });

  @override
  State<WelcomeScreenFrb> createState() => _WelcomeScreenFrbState();
}

class _WelcomeScreenFrbState extends State<WelcomeScreenFrb> {
  /// Read once. It is a bridge call and it answers the same thing every time,
  /// so it must not sit in a build path.
  late final String _version = _readVersion();

  static String _readVersion() {
    try {
      return lumitProductVersion();
    } catch (_) {
      return '';
    }
  }

  /// Escape closes the screen with nothing open (K-481) — the standard way out
  /// of anything that has taken the window, and the reason the shell behind it
  /// has something to show. Registered globally, as every other Escape in the
  /// shell is (`fx_console_frb`): this page has no focused field to route one.
  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_escapeCloses);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_escapeCloses);
    super.dispose();
  }

  bool _escapeCloses(KeyEvent event) {
    if (event is! KeyDownEvent ||
        event.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    widget.onDone();
    return true;
  }

  /// Follow a link, saying so in the status line when the desktop will not take
  /// it — the same courtesy the Help menu's rows pay.
  Future<void> _link(LumitState app, String url) async {
    if (await openExternalLink(url)) return;
    app.postNotice(l10n.couldNotOpenLink(url), error: true);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final app = context.read<LumitState>();
    final ui = context.watch<LumitUiState>();
    final workspace = ui.workspace;

    return ColoredBox(
      color: t.surface0,
      child: LayoutBuilder(
        builder: (context, box) => SingleChildScrollView(
          child: ConstrainedBox(
            constraints: BoxConstraints(minHeight: box.maxHeight),
            child: Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: welcomeBlockGap),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    // The wordmark, not a phrase: a brand mark is the same in
                    // every language, and since K-480 it is the website's own
                    // lockup — the blue key, `umi`, the violet key — rather
                    // than the word set in mono.
                    LumitWordmark(
                      key: const ValueKey('welcome-wordmark'),
                      height: welcomeWordmarkHeight,
                      // The page's own ground, which is what the lettering has
                      // to be read against.
                      ground: t.surface0,
                    ),
                    const SizedBox(height: welcomeBlockGap),
                    _cards(),
                    const SizedBox(height: welcomeBlockGap),
                    _recents(app, workspace),
                    const SizedBox(height: welcomeBlockGap),
                    _footer(t, app),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// The three ways to start: give the project a home now, start without one,
  /// or open one that exists.
  Widget _cards() => SizedBox(
        width: welcomeColumnWidth,
        child: StartCards(
          onStarted: widget.onDone,
          openPicker: widget.openPicker,
          savePicker: widget.savePicker,
        ),
      );

  Widget _recents(LumitState app, Workspace workspace) {
    final t = ThemeScope.of(context).theme;
    final paths = workspace.recentProjects;
    return SizedBox(
      width: welcomeColumnWidth,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox(
            key: const ValueKey('welcome-recent-header'),
            height: welcomeRecentHeaderHeight,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2),
              child: Row(
                children: [
                  Text(l10n.welcomeRecent.toUpperCase(), style: t.kicker),
                  const Spacer(),
                  // No question first. The one destructive control that asks
                  // is the disk cache, because that one throws away a night's
                  // rendering with nothing to undo (shell/cache_confirm_frb);
                  // a list of paths costs a trip to File ▸ Open to rebuild.
                  _Quiet(
                    id: 'welcome-clear-recent',
                    label: l10n.welcomeClearRecent,
                    onTap: paths.isEmpty ? null : workspace.clearRecentProjects,
                  ),
                ],
              ),
            ),
          ),
          // A Container, not a DecoratedBox: a decoration's border has to
          // *inset* the rows, or the well measures its content and paints its
          // hairline over the first and last row rather than around them.
          Container(
            key: const ValueKey('welcome-recent-well'),
            decoration: BoxDecoration(
              color: t.surface1,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              border: Border.all(color: t.hairline),
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (paths.isEmpty)
                  SizedBox(
                    key: const ValueKey('welcome-recent-empty'),
                    height: welcomeRecentRowHeight,
                    child: Padding(
                      padding: welcomeRecentRowPadding,
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: Text(l10n.welcomeNoRecent, style: t.body),
                      ),
                    ),
                  )
                else
                  for (var i = 0; i < paths.length; i++)
                    _RecentRow(
                      key: ValueKey<String>('welcome-recent-row-$i'),
                      index: i,
                      path: paths[i],
                      openedAt: workspace.recentOpenedAt(paths[i]),
                      last: i == paths.length - 1,
                      onOpen: () {
                        app.openProject(paths[i]);
                        widget.onDone();
                      },
                      onForget: () => workspace.forgetProject(paths[i]),
                    ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// The version at the left, the two links at the right. No filled action
  /// anywhere on this screen — the drawing spends none of the accent on it, and
  /// §3.1's rule is a ceiling of one, not a floor.
  Widget _footer(LumitTheme t, LumitState app) => SizedBox(
        key: const ValueKey('welcome-footer'),
        width: welcomeColumnWidth,
        height: welcomeFooterHeight,
        child: Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Row(
            children: [
              Text(_version, style: welcomeNote(t)),
              const Spacer(),
              _OutlineLink(
                id: 'welcome-manual',
                label: l10n.welcomeManual,
                onTap: () => _link(app, lumitDocsUrl),
              ),
              const SizedBox(width: 14),
              _OutlineLink(
                id: 'welcome-whats-new',
                label: l10n.welcomeWhatsNew,
                onTap: () => _link(app, lumitReleasesUrl),
              ),
            ],
          ),
        ),
      );
}

// --- The three ways to start work -----------------------------------------

/// New project: choose where the `.lum` lives **first**, then open the editor
/// on the project that now has a home (K-480).
///
/// The card's own note is the promise ("choose a project folder"), so a
/// cancelled picker leaves everything as it was rather than dropping somebody
/// into an editor they did not ask for — [onStarted] runs only when the file
/// is genuinely on disk.
Future<void> startNewProject(
  LumitState app,
  LumitUiState ui, {
  Future<String?> Function()? picker,
  VoidCallback? onStarted,
}) async {
  await saveProjectFrb(app, ui, forcePicker: true, picker: picker);
  if (app.project?.path() != null) onStarted?.call();
}

/// Blank project: no file, no questions.
///
/// From the welcome screen there is nothing to make — the empty project the
/// application boots with is already loaded (see `main`), so this is the card
/// that says "get out of my way". From the empty shell, where the project may
/// be one that *does* have a home on disk with nothing shown in it, it starts a
/// fresh one, which is what the word blank means.
void startBlankProject(LumitState app, {VoidCallback? onStarted}) {
  if (app.project?.path() != null) app.newProject();
  onStarted?.call();
}

/// Open: the very call File ▸ Open makes.
Future<void> startOpenProject(
  LumitState app, {
  Future<String?> Function()? picker,
  VoidCallback? onStarted,
}) async {
  await openProjectFrb(app, picker: picker);
  if (app.project?.path() != null) onStarted?.call();
}

/// The three start cards, in a row that gives each of them a third of whatever
/// width it is handed.
///
/// One widget for the two places they appear — the welcome screen and the empty
/// shell behind it — so the ways to start work cannot drift apart. [onStarted]
/// is what a card does once it has put a document up: on the welcome that hands
/// the window to the shell, and in the shell there is nothing left to do.
class StartCards extends StatelessWidget {
  final VoidCallback? onStarted;
  final Future<String?> Function()? openPicker;
  final Future<String?> Function()? savePicker;

  const StartCards({
    super.key,
    this.onStarted,
    this.openPicker,
    this.savePicker,
  });

  @override
  Widget build(BuildContext context) {
    final app = context.read<LumitState>();
    final ui = context.read<LumitUiState>();
    return Row(
      children: [
        Expanded(
          child: _WelcomeCard(
            id: 'welcome-card-new',
            title: l10n.welcomeNewProject,
            note: l10n.welcomeNewProjectNote,
            onTap: () => startNewProject(app, ui,
                picker: savePicker, onStarted: onStarted),
          ),
        ),
        const SizedBox(width: welcomeCardGap),
        Expanded(
          child: _WelcomeCard(
            id: 'welcome-card-blank',
            title: l10n.welcomeBlankProject,
            note: l10n.welcomeBlankProjectNote,
            onTap: () => startBlankProject(app, onStarted: onStarted),
          ),
        ),
        const SizedBox(width: welcomeCardGap),
        Expanded(
          child: _WelcomeCard(
            id: 'welcome-card-open',
            title: l10n.welcomeOpenProject,
            note: l10n.welcomeOpenProjectNote,
            onTap: () =>
                startOpenProject(app, picker: openPicker, onStarted: onStarted),
          ),
        ),
      ],
    );
  }
}

/// What the Viewer shows when there is nothing to show (K-481).
///
/// In plain terms: the welcome screen can be closed with nothing open, so
/// something has to be behind it — and an empty editor whose largest panel says
/// "select a composition" when there is no composition to select is a dead end.
/// The three ways to start work stand there instead, until something is
/// displayed.
///
/// A project that *has* compositions and simply has none fronted is a different
/// sentence, and keeps the panel's ordinary empty line.
class EmptyStageFrb extends StatelessWidget {
  /// The pickers, so a widget test can answer them — the same seam the welcome
  /// screen takes.
  final Future<String?> Function()? openPicker;
  final Future<String?> Function()? savePicker;

  const EmptyStageFrb({super.key, this.openPicker, this.savePicker});

  @override
  Widget build(BuildContext context) {
    // Watched: the moment a composition exists this is the wrong thing to be
    // showing. `comps()` is the cached list (K-184), so a rebuild asks the
    // engine nothing.
    final app = context.watch<LumitState>();
    if (app.comps().isNotEmpty) {
      return PlaceholderPanel(
        icon: LumitIcon.footage,
        title: l10n.panelViewer,
        hint: l10n.selectACompositionFirst,
      );
    }
    return Center(
      key: const ValueKey('empty-stage'),
      child: Padding(
        padding: const EdgeInsets.all(welcomeBlockGap),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: welcomeColumnWidth),
          child: StartCards(
            openPicker: openPicker,
            savePicker: savePicker,
          ),
        ),
      ),
    );
  }
}

/// One start card: a title, a note under it, and the whole thing is the button
/// — the note is as much a part of the choice as the words at the top of it.
class _WelcomeCard extends StatefulWidget {
  final String id;
  final String title;
  final String note;
  final VoidCallback onTap;

  const _WelcomeCard({
    required this.id,
    required this.title,
    required this.note,
    required this.onTap,
  });

  @override
  State<_WelcomeCard> createState() => _WelcomeCardState();
}

class _WelcomeCardState extends State<_WelcomeCard> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: ValueKey<String>(widget.id),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: Container(
          height: welcomeCardHeight,
          padding: welcomeCardPadding,
          decoration: BoxDecoration(
            // Resting is the drawing's own: the card sits a step above the
            // page in `surface_1` behind a plain hairline. Hover takes both up
            // one step, which is the same grammar every house button uses.
            color: _hover ? t.surface2 : t.surface1,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: _hover ? t.hairlineStrong : t.hairline),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                widget.title,
                style: t.bodyPrimary.copyWith(fontSize: 13),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
              const SizedBox(height: 4),
              Text(
                widget.note,
                style: welcomeNote(t),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// One recent project: how it looked when it was last saved, what it is
/// called, where it lives, when it was last opened here, and the × that
/// forgets it.
class _RecentRow extends StatefulWidget {
  final int index;
  final String path;
  final DateTime? openedAt;
  final bool last;
  final VoidCallback onOpen;
  final VoidCallback onForget;

  const _RecentRow({
    super.key,
    required this.index,
    required this.path,
    required this.openedAt,
    required this.last,
    required this.onOpen,
    required this.onForget,
  });

  @override
  State<_RecentRow> createState() => _RecentRowState();
}

class _RecentRowState extends State<_RecentRow> {
  bool _hover = false;

  /// The project's saved picture, or null when it has none — asked for **once**
  /// rather than per build. The row rebuilds on every hover, and a `stat` per
  /// rebuild would put the file system in a pointer path.
  late final File? _thumb = _savedThumbnail(widget.path);

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
          height:
              widget.last ? welcomeRecentRowHeight : welcomeRecentRowHeight + 1,
          padding: welcomeRecentRowPadding,
          decoration: BoxDecoration(
            color: _hover ? t.surface2 : null,
            // The seam between rows, and none under the last: the well's own
            // border closes it off.
            border: widget.last
                ? null
                : Border(bottom: BorderSide(color: t.hairline)),
          ),
          child: Row(
            children: [
              _RecentThumb(
                key: ValueKey<String>('welcome-recent-thumb-${widget.index}'),
                file: _thumb,
              ),
              const SizedBox(width: welcomeThumbGap),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisAlignment: MainAxisAlignment.center,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      projectDisplayName(widget.path),
                      style: t.bodyPrimary,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    const SizedBox(height: 2),
                    Text(
                      shortenHomePath(widget.path),
                      style: t.mono.copyWith(
                        fontSize: 9,
                        color: t.textDisabled,
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ],
                ),
              ),
              const SizedBox(width: welcomeNameGap),
              SizedBox(
                width: welcomeDateColumnWidth,
                child: Text(
                  recentWhen(widget.openedAt),
                  style: welcomeFact(t),
                  textAlign: TextAlign.right,
                  maxLines: 1,
                ),
              ),
              const SizedBox(width: welcomeForgetGap),
              _Forget(
                id: 'welcome-recent-close-${widget.index}',
                onTap: widget.onForget,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The project's saved picture, if it has one (K-468).
///
/// A file that is simply not there is the ordinary case, not an error: a
/// project last saved before this feature existed has none, and neither does
/// one whose capture failed or whose file has since been moved — the key is the
/// path, so moving a project loses its picture until the next save.
File? _savedThumbnail(String path) {
  try {
    final f = Workspace.thumbnailFile(path);
    return f.existsSync() ? f : null;
  } catch (_) {
    return null;
  }
}

/// The picture at the head of a recent row — 16:9, sized to the row.
///
/// With no file it is a quiet well with the composition mark muted in it: the
/// same `surface_0` inset the rest of the interface uses for "nothing here
/// yet", and no words, because a row that has to explain its own empty picture
/// is a row that has stopped being a list.
class _RecentThumb extends StatelessWidget {
  final File? file;

  const _RecentThumb({super.key, required this.file});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final f = file;
    final nothing = Center(
      key: const ValueKey('welcome-recent-thumb-empty'),
      child: lumitIcon(LumitIcon.comp, size: 14, color: t.textDisabled),
    );
    return ClipRRect(
      borderRadius: BorderRadius.circular(t.tokens.controlRadius),
      child: Container(
        width: welcomeThumbWidth,
        height: welcomeThumbHeight,
        color: t.surface0,
        child: f == null
            ? nothing
            : Image.file(
                f,
                fit: BoxFit.cover,
                filterQuality: FilterQuality.medium,
                // A file that vanished between the check and the decode, or
                // was written half through, falls back rather than throwing:
                // the picture is a nicety and the row still opens the project.
                errorBuilder: (_, __, ___) => nothing,
              ),
      ),
    );
  }
}

/// The × that takes one project off the list.
///
/// Its own gesture detector inside the row's, opaque, so the arena hands the
/// tap to the innermost hit and pressing the × never also opens the project.
/// No question asked: one forgotten row is a trip to File ▸ Open to get back.
class _Forget extends StatefulWidget {
  final String id;
  final VoidCallback onTap;

  const _Forget({required this.id, required this.onTap});

  @override
  State<_Forget> createState() => _ForgetState();
}

class _ForgetState extends State<_Forget> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: l10n.welcomeForgetProject,
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          key: ValueKey<String>(widget.id),
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          child: SizedBox(
            width: welcomeForgetColumnWidth,
            height: welcomeRecentRowHeight,
            child: Center(
              // The composition tabs' own close mark (timeline_extras_frb):
              // muted, no box, and it brightens under the pointer.
              child: Text(
                '×',
                style: t.body.copyWith(
                  color: _hover ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A word that acts and draws nothing — Clear, beside the recents heading.
class _Quiet extends StatefulWidget {
  final String id;
  final String label;
  final VoidCallback? onTap;

  const _Quiet({required this.id, required this.label, this.onTap});

  @override
  State<_Quiet> createState() => _QuietState();
}

class _QuietState extends State<_Quiet> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final enabled = widget.onTap != null;
    return MouseRegion(
      cursor: enabled ? SystemMouseCursors.click : MouseCursor.defer,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: ValueKey<String>(widget.id),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: Text(
          widget.label,
          style: welcomeNote(t).copyWith(
            color: !enabled
                ? t.textDisabled
                : _hover
                    ? t.textPrimary
                    : t.textMuted,
          ),
        ),
      ),
    );
  }
}

/// One of the footer's two outlined links. The house button's resting face —
/// an outline over the page's own surface — at the drawing's 24 and with its
/// label in the secondary grey rather than the primary one.
class _OutlineLink extends StatelessWidget {
  final String id;
  final String label;
  final VoidCallback onTap;

  const _OutlineLink({
    required this.id,
    required this.label,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      height: welcomeButtonHeight,
      child: HouseButton(
        key: ValueKey<String>(id),
        padding: const EdgeInsets.symmetric(horizontal: 10),
        onPressed: onTap,
        // `widthFactor` keeps the button as wide as its word while letting it
        // fill the 24 the drawing gives it.
        child: Align(widthFactor: 1, child: Text(label, style: t.body)),
      ),
    );
  }
}

// --- What a row says about a project --------------------------------------

/// A project's name: the file's, without the extension. The engine has no other
/// name for a document, and the file is what the user called it.
String projectDisplayName(String path) {
  final name = path.split(RegExp(r'[/\\]')).last;
  return name.toLowerCase().endsWith('.lum')
      ? name.substring(0, name.length - 4)
      : name;
}

/// The path with the user's home folder written as `~`, and back-slashes
/// forward, as the drawing sets it. Presentation only — the stored path is
/// untouched, and it is the stored one that is opened.
String shortenHomePath(String path) {
  final home =
      Platform.environment['USERPROFILE'] ?? Platform.environment['HOME'];
  final forward = path.replaceAll('\\', '/');
  if (home == null || home.isEmpty) return forward;
  final root = home.replaceAll('\\', '/');
  return forward.toLowerCase().startsWith(root.toLowerCase())
      ? '~${forward.substring(root.length)}'
      : forward;
}

/// When the project was last opened on this machine: today says so in words,
/// anything older is the day and the month.
String recentWhen(DateTime? when) {
  if (when == null) return '';
  final now = DateTime.now();
  final sameDay =
      when.year == now.year && when.month == now.month && when.day == now.day;
  return sameDay ? l10n.welcomeToday : l10n.welcomeRecentDate(when);
}
