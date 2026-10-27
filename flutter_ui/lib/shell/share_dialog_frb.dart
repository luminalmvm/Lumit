// The two windows of a shared project, on the flutter_rust_bridge API. The
// Shared project window is the one place sharing is driven from: it shares the
// open project or joins somebody else's, and once either has happened it lists
// who is here and holds what a host or a guest can do about it. The Conflicts
// window settles what a guest changed while it was away.
//
// The engine does the sharing. These hold the fields and show its answers, and
// what they read from it is read when a window opens or a button is pressed,
// never in a build.
//
// The frame is the dialog pattern's: title strip, body, footer.

import 'dart:async';
import 'dart:io' show File, FileSystemException, Platform;
import 'dart:math' as math;
import 'dart:typed_data' show Float32List;
import 'dart:ui' show PointMode;

import 'package:flutter/material.dart' show SelectableText;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:intl/intl.dart' show DateFormat;
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart'
    show BridgeCompSettings;
import 'package:lumit_flutter/src/rust/api/share.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/file_dialogs.dart';
import '../state/friend_invites.dart';
import '../state/online_footage.dart';
import '../state/share.dart';
import '../state/timecode.dart';
import '../state/updates.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/share_marks.dart';
import 'dialog_frame.dart';
import 'menu_bar_frb.dart' show saveProjectFrb;
import 'pro_window.dart';
import 'profile_chip.dart';
import 'review_link_parts.dart' show reviewRowMenu;
import 'project_history_window.dart'
    show backupDay, backupSize, backupTime;
import 'settings_window_frb.dart';
import 'sign_in_window.dart';
import 'update_dialog_frb.dart';
import 'welcome_frb.dart' show projectDisplayName;

/// The width both take, the column their labels sit in, and how tall the list
/// of conflicts and the list of footage grow before they scroll.
const double shareDialogWidth = 400;
const double _labelColumn = 100;
const double _conflictListHeight = 320;

/// A single-line row of a list in this window, and how far the lists of
/// people and of footage grow before they scroll. Each stops half a row in,
/// so the row cut by the edge says there is more.
const double _listRow = 24;
const double _peopleListHeight = _listRow * 6.5;
const double _footageListHeight = _listRow * 5.5;

/// How tall the list of who changed what grows before it scrolls, which is
/// four and a half sittings, and how many it shows until it is asked for
/// them all.
const double _activityListHeight = 153;
const int _activityShown = 12;

/// The box a glyph that leads a line sits in, and the gap to its words, so
/// every such line in the window starts its words at one edge.
const double _leadGlyph = 14;
const double _leadGap = 6;

/// The room the title strip, the footer and the edges of the window take,
/// which the body of a project kept online is kept inside of.
const double _aroundBody = 180;

/// What Lumit's server said no with, about a project kept online, in words.
String shareOnlineErrorText(String code) => switch (code) {
      'pro_required' => l10n.shareOnlineErrorPro,
      'too_many' => l10n.shareOnlineErrorTooMany,
      'signed_out' || 'unauthorized' => l10n.shareOnlineErrorSignedOut,
      'offline' => l10n.signInErrorOffline,
      'slow_down' => l10n.signInErrorSlow,
      _ => l10n.shareOnlineErrorRefused,
    };

/// What is in the way of a project kept online being in step, for the first
/// line of its window and for the strip. Null when it is in step. Said in
/// the order one causes the next: a machine on another Lumit, or one the
/// server takes no edits from, is away because of it.
///
/// One rule for every state. [words] names it, and is the only thing drawn
/// in the warning colour, which it is whenever edits are not being shared
/// or are about to stop ([wrong]). [line] goes under it in the window,
/// quietly, and adds to it without saying it again. [said] is the whole of
/// it in one sentence, for the strip, which has no state word beside it to
/// lean on.
({String words, String line, String said, bool wrong})? shareTrouble(
    ShareState share) {
  final hosting = share.online?.host ?? share.role == ShareRole.host;
  if (share.forwarding) {
    final moving = l10n.shareForwardMoving;
    // A moment's work, not a state to be warned of.
    return (words: moving, line: moving, said: moving, wrong: false);
  }
  switch (share.versions) {
    case BridgeShareVersions_Movable(:final version):
      final line = l10n.shareVersionsMovable;
      return (
        words: l10n.shareVersionsOlder(version),
        line: line,
        said: line,
        wrong: true,
      );
    case BridgeShareVersions_HostFirst():
      final line = l10n.shareVersionsHostFirst(shareThisLumit());
      return (
        words: l10n.shareVersionsWaitingHost,
        line: line,
        said: line,
        wrong: true,
      );
    case BridgeShareVersions_Update(:final version):
      return (
        words: l10n.shareVersionsNeedsUpdate,
        line: l10n.shareVersionsUpdateLine(version),
        said: l10n.shareVersionsUpdate(version),
        wrong: true,
      );
    case null:
  }
  // The project has grown past what the server stores, which this machine
  // found out by going to store it.
  final large = share.online?.tooLarge ?? false;
  if (large && !share.writable) {
    return (
      words: l10n.shareOnlineTooLarge,
      // Only the host can do anything about it.
      line: hosting
          ? l10n.shareOnlineTooLargeLineOwn
          : l10n.shareOnlineTooLargeLine,
      said: l10n.shareOnlineTooLargeSaid,
      wrong: true,
    );
  }
  if (share.online != null && !share.writable) {
    final line =
        hosting ? l10n.shareOnlineWaitingOwn : l10n.shareOnlineWaiting;
    return (
      words: l10n.shareOnlineOnHold,
      line: line,
      said: line,
      wrong: true,
    );
  }
  if (share.replaced) {
    // Not looking for the server any more: it has said the invite is gone.
    return (
      words: l10n.shareReplaced,
      line: l10n.shareReplacedLine,
      said: l10n.shareReplacedSaid,
      wrong: true,
    );
  }
  if (!share.away) {
    // Edits still go in, and will stop unless it gets smaller.
    return large
        ? (
            words: l10n.shareOnlineNearlyLarge,
            line: l10n.shareOnlineNearlyLargeLine,
            said: l10n.shareOnlineNearlyLargeSaid,
            wrong: true,
          )
        : null;
  }
  return share.online != null
      ? (
          words: l10n.shareOnlineLooking,
          line: l10n.shareOnlineAwayLine,
          said: l10n.shareOnlineAway,
          wrong: true,
        )
      : (
          words: l10n.shareHostUnreachable,
          line: l10n.shareAway,
          said: l10n.shareAway,
          wrong: true,
        );
}

/// Ask before a project stops being kept online: one question, for its own
/// window and for the list in Settings. [name] is the project's, where the
/// asker lists several. [here] is whether a copy of it is on this computer.
Future<bool> askStopKeeping(BuildContext context,
        {String? name, bool here = true}) async =>
    await showLumitModal<bool>(
      context: context,
      id: 'share-stop-ask',
      builder: (close) => _Question(
        title: name == null
            ? l10n.shareOnlineStopTitle
            : l10n.shareOnlineStopTitleNamed(name),
        body: here ? l10n.shareOnlineStopBody : l10n.shareOnlineStopBodyAway,
        no: l10n.shareOnlineStopNo,
        yes: l10n.settingsOnlineStop,
        keyPrefix: 'share-stop-ask',
        onChoose: close,
      ),
    ) ==
    true;

/// The relay to share through: the person's [own], or with none and Pro,
/// Lumit's, which is reached through a door that has to be open first.
Future<String?> shareRelayFor(LumitUiState ui, String own) async {
  if (own.isNotEmpty) return own;
  if (ui.workspace.shareCloud &&
      ui.account.pro &&
      await ui.relayDoor.open()) {
    return shareCloudRelay();
  }
  return null;
}

/// Share the open project for the extension called [extension], once the
/// person agrees, and answer the invite. It is shared the way the person
/// last set up: their name, relay and router choice. The invite is a new
/// one that is not kept, so whoever the extension sends it to is out once
/// sharing stops. Null when the person said no or it could not start.
Future<String?> shareForExtension(
    BuildContext context, LumitState app, String extension,
    {String? password}) async {
  // Share only takes a press once the question has been up a moment, so a
  // click the page lined up lands on nothing.
  final ready = Future<void>.delayed(const Duration(milliseconds: 600));
  final yes = await showLumitModal<bool>(
        context: context,
        id: 'share-extension-ask',
        builder: (close) => FutureBuilder<void>(
          future: ready,
          builder: (context, wait) => _Question(
            title: l10n.shareExtensionTitle(extension),
            body: l10n.shareExtensionBody(extension),
            no: l10n.shareExtensionNo,
            yes: l10n.shareExtensionYes,
            keyPrefix: 'share-extension-ask',
            yesLive: wait.connectionState == ConnectionState.done,
            onChoose: close,
          ),
        ),
      ) ==
      true;
  if (!yes || !context.mounted) return null;
  final ui = context.read<LumitUiState>();
  final prefs = ui.workspace;
  final relay = await shareRelayFor(ui, prefs.shareRelay ?? '');
  final started = app.startSharing(
      name: prefs.shareName ?? l10n.shareDefaultName,
      port: shareDefaultPort(),
      password: password,
      outside: prefs.shareOutside,
      relay: relay);
  return started is BridgeShareStarted_Sharing ? app.share.link() : null;
}

/// Ask the host whether to move a project kept online forward to this
/// Lumit, which cannot be taken back. [version] is the Lumit that keeps it.
Future<bool> askForward(BuildContext context, String version) async =>
    await showLumitModal<bool>(
      context: context,
      id: 'share-forward-ask',
      builder: (close) => _Question(
        title: l10n.shareForwardTitle(shareThisLumit()),
        body: l10n.shareForwardBody(version),
        no: l10n.shareForwardNo,
        yes: l10n.shareForwardYes,
        keyPrefix: 'share-forward-ask',
        onChoose: close,
      ),
    ) ==
    true;

/// Open [copy], this machine's own project from just before a return, as a
/// project of its own that joins nothing. The engine keeps it in Lumit's own
/// folder, so it is first copied out the way Project history copies a
/// version out: beside the shared project's file, so its footage is found,
/// or where the person says when that has no file. [close] shuts whatever
/// window asked, once there is a file to open. [picker] stands in for the
/// system's save window, as it does for saving.
Future<void> openReturnCopy(BuildContext context, LumitState app,
    BridgeShareReturnCopy copy, VoidCallback close,
    {Future<String?> Function()? picker}) async {
  final from = copy.path;
  final kept = File(from);
  String? path;
  try {
    path = app.project?.path();
  } catch (_) {
    // Closed already.
  }
  String? target;
  try {
    target = path == null
        ? await (picker ?? pickProjectSaveLocation)()
        : '${File(path).parent.path}${Platform.pathSeparator}'
            '${projectDisplayName(path)} '
            '(${DateFormat('yyyy-MM-dd HH.mm').format(_copyAt(copy))}).lum';
    if (target == null) return;
    if (!target.toLowerCase().endsWith('.lum')) target = '$target.lum';
    // Copied out once. Opened a second time it is the file the person may
    // have worked on since, which a fresh copy must not write over. Where
    // they chose the place themselves, the system asked about that.
    if (path == null || !await File(target).exists()) await kept.copy(target);
  } on FileSystemException {
    app.postNotice(l10n.couldNotOpen(from), error: true);
    return;
  }
  close();
  await app.openProject(target);
}

/// When a copy from before a return was kept.
DateTime _copyAt(BridgeShareReturnCopy copy) =>
    DateTime.fromMillisecondsSinceEpoch(copy.at * 1000);

/// Update Lumit by the road the Help menu has: one press checks, and goes
/// straight on to offering what it found.
Future<void> offerLumitUpdate(
    BuildContext context, LumitState app, LumitUiState ui) async {
  final updates = ui.updates;
  Future<void> press() => pressUpdateRow(
        context,
        updates: updates,
        notice: app.postNotice,
        projectIsDirty: () => app.project?.isDirty() ?? false,
        saveProject: () => saveProjectFrb(app, ui, leaving: true),
      );
  final found = updates.stage == UpdateStage.available ||
      updates.stage == UpdateStage.ready;
  await press();
  if (!found && updates.stage == UpdateStage.available && context.mounted) {
    await press();
  }
}

/// A text well at the dialog's control height, filling its row.
Widget _field(
  LumitTheme t,
  String id,
  TextEditingController controller, {
  String? hint,
  VoidCallback? onSubmitted,
  TextStyle? style,
  bool obscure = false,
}) =>
    SizedBox(
      height: dialogControlHeight,
      child: HouseTextField(
        key: ValueKey<String>(id),
        controller: controller,
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        fill: t.surface0,
        style: style,
        hint: hint,
        obscure: obscure,
        onSubmitted: onSubmitted == null ? null : (_) => onSubmitted(),
      ),
    );

/// A quiet line of explanation, or what went wrong in the warning colour.
Widget _line(LumitTheme t, String text, {bool warning = false}) => Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Text(text,
          style: t.small.copyWith(color: warning ? t.warning : t.textMuted)),
    );

/// The body between the title strip and the footer.
Widget _body(List<Widget> children) => Padding(
      padding: const EdgeInsets.all(dialogPadding),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children,
      ),
    );

// --- Shared project -------------------------------------------------------

/// Open the Shared project window: share the open project, join somebody
/// else's, or see who is here. [invite] is a link to open it on the Join
/// page with, as a click on one outside Lumit brings. [relay] is a relay to
/// offer on the Share page, as an extension with one of its own asks.
Future<void> showShareFrb(BuildContext context, LumitState app,
        {String? invite,
        String? relay,
        bool activity = false,
        Future<String?> Function()? savePicker}) =>
    showLumitModal<void>(
      context: context,
      id: 'share',
      builder: (close) => _ShareDialog(
          app: app,
          invite: invite,
          relay: relay,
          activity: activity,
          savePicker: savePicker,
          onClose: () => close(null)),
    );

/// The two things there are to do before a project is shared.
enum _Page { share, join }

class _ShareDialog extends StatefulWidget {
  final LumitState app;
  final String? invite;
  final String? relay;

  /// Opened to see who changed what, which is brought into view.
  final bool activity;

  /// Stands in for the system's save window, as it does for saving.
  final Future<String?> Function()? savePicker;
  final VoidCallback onClose;

  const _ShareDialog(
      {required this.app,
      this.invite,
      this.relay,
      this.activity = false,
      this.savePicker,
      required this.onClose});

  @override
  State<_ShareDialog> createState() => _ShareDialogState();
}

class _ShareDialogState extends State<_ShareDialog> {
  late final LumitUiState _ui;
  late final Workspace _prefs;
  late final TextEditingController _name;
  late final TextEditingController _port;
  late final TextEditingController _relay;
  late final int _defaultPort;

  /// One more address this machine can be reached at, which only the person
  /// knows of: a VPN's, or a port they forwarded by hand.
  final TextEditingController _address = TextEditingController();

  /// What a guest pastes: the link to join by, and a fresh one for a host
  /// that has moved.
  late final TextEditingController _joinInvite;
  final TextEditingController _newInvite = TextEditingController();

  /// The password a host sets, or a guest gives with a link that needs one.
  final TextEditingController _password = TextEditingController();

  /// How fast this computer sends and takes footage, in kilobytes a second.
  late final TextEditingController _upLimit;
  late final TextEditingController _downLimit;

  /// The link in the field wants a password with it.
  bool _needsPassword = false;

  /// The friend's invite whose link is in the Join field, put away once it
  /// has been joined by.
  ({String id, String link})? _fromFriend;

  /// The password is being shown as it is typed.
  bool _passwordShown = false;

  _Page _page = _Page.share;

  /// What the engine knows the open project by, which the secret of its
  /// invite is filed under, and that secret if it was shared from here before.
  String? _projectId;
  String? _keptKey;

  /// This time's invite is the one handed out last time.
  bool _sameInvite = false;

  /// Whether sharing asks the router to open the port.
  late bool _outside;

  /// The rows most people never need are showing.
  bool _advanced = false;

  /// The invite link, while this machine hosts.
  String? _link;

  /// The person asked to see the links this window holds. Until then they
  /// are not drawn: a link is all it takes to join, and windows end up on
  /// streams and in screenshots.
  bool _shown = false;

  /// Where this machine keeps the footage of a project it is joining.
  String? _footage;

  /// The host is being asked and the project fetched.
  bool _joining = false;

  /// The invite that can only look is being shown, as [_shown] says of the
  /// other.
  bool _shownLook = false;

  /// Why the last thing asked for did not happen, to show under the fields.
  String? _error;

  /// Lumit's server is being asked something that has not come back: to
  /// keep the project, to take someone out, or to stop keeping it.
  bool _busy = false;

  /// The invite of a project kept online was replaced by taking someone out.
  bool _newLink = false;

  /// The invite in hand wants a password with it. Read when the link is,
  /// and never in a build.
  bool _linkLocked = false;

  /// This machine keeps the open project online as its host. Remembered
  /// here as well as asked of the server, which a copy opened again has not
  /// found yet, so its host is never offered a way to leave it behind.
  ///
  /// Never a machine that can only look, whoever is signed in on it: it can
  /// take nobody out, make no invite and move nothing.
  bool _hosting(ShareState share) =>
      !(share.online?.look ?? false) &&
      ((share.online?.host ?? false) ||
          (_prefs.shareHosted[_projectId]?.startsWith('online') ?? false));

  /// The sittings that were new when this window showed them. They keep
  /// their place above the rule for as long as it is open. The engine was
  /// told at once that they have been looked at.
  final Set<String> _fresh = {};

  /// Every sitting is listed, and not only the newest.
  bool _allActivity = false;

  /// The Share page's tick: keep the project online, and not share it from
  /// this machine.
  bool _online = false;

  /// The window is taller than the screen has room for, so its body scrolls
  /// and the lists inside it are let grow: one scroller, never one inside
  /// another. It stays so for as long as the window is open.
  bool _oneScroller = false;

  /// Who is being taken out of a project kept online just now.
  int? _removing;

  /// What the host last asked the invite that can only look to be, until
  /// the server has said that it is.
  bool? _lookAsked;

  /// The note being written.
  final TextEditingController _note = TextEditingController();

  /// The list of who changed what: the sittings and the notes together,
  /// newest first, with the notes still waiting to be sent before them. Put
  /// together when the list is read, and never in a build.
  List<Object> _entries = const [];

  /// Where each note on show was written, in words: the time on its
  /// composition's clock, and its layer's name.
  final Map<String, ({String? timecode, String? layer})> _noteWhere = {};

  /// Someone away has asked for the field a new invite goes in.
  bool _reinviteOpen = false;

  /// The project an invite names is being moved forward to this Lumit.
  bool _moving = false;

  /// How this Lumit stands to the project the last join found, when another
  /// version keeps it and this machine cannot move it.
  BridgeShareVersions? _joinMet;

  final GlobalKey _activityKey = GlobalKey();

  static String _entryKey(Object entry) => switch (entry) {
        BridgeShareSitting(:final name, :final began) => '$name|$began',
        BridgeShareNote(:final id) => 'note|$id',
        _ => '',
      };

  /// When an entry of the list was: a sitting's last edit, or the writing
  /// of a note. A note still waiting to be sent comes before them all.
  static int _entryWhen(Object entry) => switch (entry) {
        BridgeShareSitting(:final ended) => ended,
        BridgeShareNote(:final waiting, :final at) =>
          waiting ? (1 << 61) + at : at,
        _ => 0,
      };

  /// As much of the list as is drawn until all of it is asked for.
  List<Object> get _shownEntries =>
      _allActivity || _entries.length <= _activityShown
          ? _entries
          : _entries.sublist(0, _activityShown);

  /// The list of who changed what was read again. Whatever in it is new has
  /// now been looked at.
  void _lookAtActivity() {
    final share = widget.app.share;
    _entries = [...share.activity, ...share.notes]
      ..sort((a, b) => _entryWhen(b).compareTo(_entryWhen(a)));
    _readNoteWhere();
    final fresh = [
      for (final sitting in share.activity)
        if (sitting.isNew) _entryKey(sitting),
      for (final note in share.notes)
        if (note.isNew) _entryKey(note),
    ];
    if (fresh.isEmpty) return;
    _fresh.addAll(fresh);
    unawaited(widget.app.activitySeen());
  }

  /// Ask the engine where each note on show was written, once a note.
  void _readNoteWhere() {
    final rates = <String, BridgeCompSettings>{};
    for (final note in _shownEntries.whereType<BridgeShareNote>()) {
      if (_noteWhere.containsKey(note.id)) continue;
      final (place, frame) = (note.comp, note.frame);
      try {
        final comp = place?.comp;
        final rate =
            comp == null ? null : rates[place!.id] ??= comp.getSettings();
        _noteWhere[note.id] = (
          timecode: rate == null || frame == null
              ? null
              : timecodeOfRate(frame, rate.fpsNum, rate.fpsDen),
          layer: note.layer?.getName(),
        );
      } catch (_) {
        // Gone since the list was read. The note is drawn without it.
      }
    }
  }

  /// Leave the note that was typed, about where this person is now.
  void _sendNote() {
    // Five hundred characters is what a note holds.
    final text = String.fromCharCodes(_note.text.trim().runes.take(500));
    final comp = _ui.selectedComp;
    if (text.isEmpty ||
        !widget.app.leaveNote(text,
            comp: comp,
            frame: comp == null ? null : _ui.playheadFrame.value,
            layer: comp == null ? null : _ui.selectedLayer.value)) {
      return;
    }
    _note.clear();
  }

  /// Go where a note was written: its composition, the frame the playhead
  /// was on, and its layer chosen.
  void _goToNote(BridgeShareNote note) {
    final comp = note.comp?.comp;
    if (comp == null) return;
    final frame = note.frame;
    if (comp.internalid != _ui.selectedComp?.internalid) {
      _ui.setSelectedComp(comp, atFrame: frame);
    } else if (frame != null) {
      _ui.scrubTo(frame);
    }
    if (note.layer case final layer?) _ui.setSelection([layer]);
    widget.onClose();
  }

  /// Turn the invite that can only look on or off, for the host. Where a
  /// password is set, turning it on lets either link show the project
  /// without it, and turning it off puts out whoever is looking. Each is
  /// asked first.
  Future<void> _turnLook(bool on) async {
    final looking = widget.app.share.others.where((p) => p.look).length;
    if (on ? _linkLocked : looking > 0) {
      final sure = await showLumitModal<bool>(
        context: context,
        id: 'share-look-ask',
        builder: (close) => _Question(
          title: on ? l10n.shareLookOnTitle : l10n.shareLookOffTitle,
          body: on ? l10n.shareLookOnBody : l10n.shareLookOffBody(looking),
          no: l10n.cancel,
          yes: on ? l10n.shareLookOnYes : l10n.shareLookOffYes,
          keyPrefix: 'share-look-ask',
          onChoose: close,
        ),
      );
      if (sure != true || !mounted) return;
    }
    setState(() {
      _lookAsked = on;
      _error = null;
      if (!on) _shownLook = false;
    });
    try {
      await _ui.setOnlineLook(on);
    } on CloudError catch (e) {
      if (!mounted) return;
      setState(() {
        _lookAsked = null;
        _error = shareOnlineErrorText(e.code);
      });
    }
  }

  @override
  void initState() {
    super.initState();
    _ui = context.read<LumitUiState>();
    _prefs = _ui.workspace;
    _defaultPort = shareDefaultPort();
    int? keptPort;
    try {
      _projectId = widget.app.project?.shareId();
    } catch (_) {
      // No project to share. The Join page needs none.
    }
    if (_prefs.shareHosted[_projectId] case final kept?
        when !kept.startsWith('online')) {
      final cut = kept.indexOf('/');
      keptPort = int.tryParse(kept.substring(0, cut < 0 ? 0 : cut));
      _keptKey = kept.substring(cut + 1);
    }
    if (widget.app.project == null || widget.invite != null) {
      _page = _Page.join;
    }
    _name = TextEditingController(text: _prefs.shareName);
    _port = TextEditingController(text: '${keptPort ?? _defaultPort}');
    _relay = TextEditingController(text: widget.relay ?? _prefs.shareRelay);
    // A relay that came with the request is shown, so the person sees what
    // their edits will pass through before they share.
    _advanced = widget.relay != null;
    _joinInvite = TextEditingController(text: widget.invite)
      ..addListener(_readInvite);
    _newInvite.addListener(_readInvite);
    _password.addListener(_redraw);
    _readInvite();
    _outside = _prefs.shareOutside;
    String limit(int kilobytes) => kilobytes == 0 ? '' : '$kilobytes';
    _upLimit = TextEditingController(text: limit(_prefs.shareUpLimit))
      ..addListener(_readLimits);
    _downLimit = TextEditingController(text: limit(_prefs.shareDownLimit))
      ..addListener(_readLimits);
    _address.addListener(_readLink);
    widget.app.share.roster.addListener(_readLink);
    _ui.friendInvites.addListener(_redraw);
    // Opening the window is a fair moment to look for a friend's invite.
    unawaited(_ui.friendInvites.check());
    _link = _linkNow();
    _readLocked();
    widget.app.share.activityRead.addListener(_lookAtActivity);
    _lookAtActivity();
    // What the host's storage holds, should the window come to say it is
    // full.
    if (widget.app.share.online?.host ?? false) {
      unawaited(_ui.backup.refresh());
    }
    // Opening the window is a fair moment to ask whether a full storage
    // has room again.
    _ui.onlineFootage.recheck();
    // It says whatever the strip was saying of the project, so that has
    // been read. After this frame, since the strip is drawn from it.
    WidgetsBinding.instance
        .addPostFrameCallback((_) => widget.app.shareWindowOpened());
    if (widget.activity) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final group = _activityKey.currentContext;
        if (group != null) Scrollable.ensureVisible(group);
      });
    }
  }

  @override
  void dispose() {
    widget.app.share.roster.removeListener(_readLink);
    _ui.friendInvites.removeListener(_redraw);
    widget.app.share.activityRead.removeListener(_lookAtActivity);
    _name.dispose();
    _port.dispose();
    _relay.dispose();
    _address.dispose();
    _joinInvite.dispose();
    _newInvite.dispose();
    _password.dispose();
    _note.dispose();
    _upLimit.dispose();
    _downLimit.dispose();
    super.dispose();
  }

  void _redraw() {
    if (mounted) setState(() {});
  }

  /// Whether the link being pasted wants a password, which is what puts the
  /// password row there.
  void _readInvite() {
    final pasted = widget.app.share.away ? _newInvite : _joinInvite;
    var locked = false;
    try {
      locked = shareLinkLocked(text: pasted.text.trim());
    } catch (_) {
      // No engine to ask, as in a widget test.
    }
    _needsPassword = locked;
    _redraw();
  }

  /// The limits as typed. Anything that is not a number is no limit.
  void _readLimits() {
    final up = int.tryParse(_upLimit.text.trim()) ?? 0;
    final down = int.tryParse(_downLimit.text.trim()) ?? 0;
    if (up != _prefs.shareUpLimit || down != _prefs.shareDownLimit) {
      _prefs.setShareLimits(up: up, down: down);
    }
  }

  /// The password typed, or null for none.
  String? _passwordNow() => _password.text.isEmpty ? null : _password.text;

  /// The invite this project was last shared by had a password, which it
  /// keeps unless another is typed.
  bool get _keptPassword => _keptKey?.contains('.') ?? false;

  /// The name the others see: what was typed, kept for next time, or the
  /// default when the field was left empty.
  String _nameNow() {
    final name = _name.text.trim();
    if (name != (_prefs.shareName ?? '')) {
      _prefs.setShareName(name.isEmpty ? null : name);
    }
    return name.isEmpty ? l10n.shareDefaultName : name;
  }

  String? _linkNow() {
    final address = _address.text.trim();
    return widget.app.share.link(address: address.isEmpty ? null : address);
  }

  /// The link holds every way to this machine the engine knows of, so it is
  /// read again when the router or a relay answers, and when an address is
  /// typed.
  void _readLink() {
    // The server has said what the invite to look was asked to be.
    if (_lookAsked == widget.app.share.online?.lookOn) _lookAsked = null;
    final link = _linkNow();
    if (link == _link || !mounted) return;
    setState(() {
      _link = link;
      _readLocked();
    });
  }

  void _readLocked() {
    try {
      _linkLocked = shareLinkLocked(text: _link ?? '');
    } catch (_) {
      // No engine to ask, as in a widget test.
      _linkLocked = false;
    }
  }

  /// Keep the open project online. That is Lumit Pro's, so someone signed
  /// out is asked to sign in first, and someone without it is shown what it
  /// is.
  Future<void> _keepOnline() async {
    final account = _ui.account;
    if (!account.signedIn && (!await showSignIn(context) || !mounted)) return;
    if (!account.pro) return unawaited(showProWindow(context));
    final (name, password) = (_nameNow(), _passwordNow());
    setState(() {
      _busy = true;
      _error = null;
    });
    await _ui.onlineDoor.open();
    final started = await widget.app.keepOnline(name: name, password: password);
    // Pro had ended since the account was last read. It is read again, so
    // the next press leads to Lumit Pro.
    if (started case BridgeOnlineStarted_Refused(code: 'pro_required')) {
      unawaited(account.refreshAccount());
    }
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = switch (started) {
        BridgeOnlineStarted_Online() => null,
        BridgeOnlineStarted_Refused(:final code) => shareOnlineErrorText(code),
        BridgeOnlineStarted_Unreachable() => l10n.signInErrorOffline,
        _ => l10n.shareCouldNotStart,
      };
      if (_error == null) _password.clear();
    });
  }

  Future<void> _start() async {
    if (_online && _ui.account.pro) return _keepOnline();
    // Anything that is not a port number asks for the usual one.
    final asked =
        (int.tryParse(_port.text.trim()) ?? _defaultPort).clamp(0, 65535);
    final own = _relay.text.trim();
    if (own != (_prefs.shareRelay ?? '')) {
      _prefs.setShareRelay(own.isEmpty ? null : own);
    }
    final relay = await shareRelayFor(_ui, own);
    if (!mounted) return;
    final started = widget.app.startSharing(
        name: _nameNow(),
        port: asked,
        key: _keptKey,
        password: _passwordNow(),
        outside: _outside,
        relay: relay);
    if (started case BridgeShareStarted_Sharing(:final port, :final key)) {
      // Kept until sharing is stopped, so the same project shared again after
      // a restart is found by the invite people already hold.
      if (_projectId case final id?) _prefs.setShareHosted(id, '$port/$key');
      _sameInvite = key == _keptKey;
      _keptKey = key;
      _password.clear();
    }
    setState(() {
      _error = switch (started) {
        BridgeShareStarted_Sharing() => null,
        BridgeShareStarted_PortInUse() => l10n.sharePortInUse(asked),
        _ => l10n.shareCouldNotStart,
      };
      _link = _linkNow();
    });
  }

  Future<void> _pickFootage() async {
    final folder = await pickFolder();
    if (folder != null && mounted) setState(() => _footage = folder);
  }

  Future<void> _join() async {
    if (_joining) return;
    setState(() {
      _joining = true;
      _error = null;
      _joinMet = null;
    });
    final password = _needsPassword ? _passwordNow() : null;
    final invite = _joinInvite.text.trim();
    var online = false;
    try {
      online = shareLinkOnline(text: invite);
    } catch (_) {
      // No engine to ask, as in a widget test.
    }
    // The host may be at Lumit's relay, which a guest needs no account for
    // but does need the door to. A project kept online has a door of its
    // own.
    await _ui.relayDoor.open();
    if (online) {
      _ui.onlineDoor.lastFailure = null;
      await _ui.onlineDoor.open();
    }
    final outcome = await widget.app.joinShared(
        invite: invite, name: _nameNow(), password: password, footage: _footage);
    if (outcome is BridgeJoinOutcome_Joined) {
      if (_fromFriend case (:final id, :final link) when link == invite) {
        unawaited(_ui.friendInvites.dismiss(id));
      }
      widget.onClose();
      return;
    }
    if (!mounted) return;
    if (outcome
        case BridgeJoinOutcome_Versions(
          versions: BridgeShareVersions_Movable(:final version)
        )) {
      // The host's own project, on an older Lumit than this one. Asked, and
      // on a yes moved forward and joined as if nothing had been in the way.
      final yes = await askForward(context, version);
      if (!mounted) return;
      String? words;
      if (yes) {
        setState(() => _moving = true);
        words = shareForwardText(
            await widget.app.forwardInvite(
                invite: invite, name: _nameNow(), password: password),
            version);
        if (!mounted) return;
      }
      setState(() {
        _moving = _joining = false;
        _error = words;
      });
      if (yes && words == null) return _join();
      return;
    }
    setState(() {
      _joining = false;
      // Another Lumit keeps it, and this machine waits or updates.
      if (outcome case BridgeJoinOutcome_Versions(:final versions)) {
        _joinMet = versions;
        return;
      }
      _error = switch (outcome) {
        // Asked about the unsaved project in hand, and kept it.
        null => null,
        BridgeJoinOutcome_BadInvite() => l10n.shareBadInvite,
        BridgeJoinOutcome_PasswordNeeded() => l10n.sharePasswordNeeded,
        // The engine hears a project that is not there and a server out of
        // reach as one answer. The door knows which it gave. A wrong
        // password finds nothing, as a wrong link does.
        BridgeJoinOutcome_Unreachable() || BridgeJoinOutcome_Failed()
            when online =>
          switch (_ui.onlineDoor.lastFailure) {
            null => l10n.shareCouldNotJoin,
            'not_found' => password != null
                ? l10n.shareOnlineNotFoundOrPassword
                : l10n.shareOnlineNotFound,
            // The server knew the invite, and why it opens nothing now.
            'closed' => l10n.shareJoinClosed,
            'moved' => l10n.shareJoinMoved,
            'removed' => l10n.shareJoinRemoved,
            'look_off' => l10n.shareJoinLookOff,
            final code => shareOnlineErrorText(code),
          },
        // A wrong password gets no answer either, so it is one of the two.
        BridgeJoinOutcome_Unreachable() when password != null =>
          l10n.shareUnreachableOrPassword,
        BridgeJoinOutcome_Unreachable() => l10n.shareUnreachable,
        BridgeJoinOutcome_VersionMismatch(:final host) =>
          l10n.shareVersionMismatch(host),
        BridgeJoinOutcome_Full() => l10n.shareFull,
        BridgeJoinOutcome_Unsafe() => l10n.shareUnsafe,
        _ => l10n.shareCouldNotJoin,
      };
    });
  }

  /// The account's friends in a menu under [button], to send the invite to:
  /// the one that can only look when [look] is set. Without a username there
  /// are none, and the menu says so.
  Future<void> _pickFriend(BuildContext button, {bool look = false}) async {
    final account = _ui.account;
    await account.loadFriends();
    if (!mounted || !button.mounted) return;
    final link = look ? widget.app.share.online?.lookLink : _link;
    if (link == null) return;
    final named = account.account?.username != null;
    final friends = named ? account.friends.friends : const <Friend>[];
    final box = button.findRenderObject()! as RenderBox;
    reviewRowMenu(button, box.localToGlobal(Offset(0, box.size.height + 2)), [
      if (!named)
        ('friends-username', l10n.friendsNeedUsername, null)
      else if (friends.isEmpty)
        ('friends-none', l10n.reviewNoFriends, null)
      else
        for (final friend in friends)
          (
            'friend-${friend.username}',
            friend.name,
            () => unawaited(_invite(friend, link, look)),
          ),
    ]);
  }

  /// Sealed for each of [friend]'s computers, with the password set here
  /// when the invite needs one, so it opens for them at a click.
  Future<void> _invite(Friend friend, String link, bool look) async {
    try {
      final sent = await _ui.friendInvites.send(friend.username, (
        link: link,
        password: look ? null : _passwordNow(),
        project: projectNameFor(widget.app.project?.path()) ?? _nameNow(),
        look: look,
      ));
      widget.app.postNotice(sent
          ? l10n.shareInviteSent(friend.name)
          : l10n.shareInviteNoComputer(friend.name));
    } on CloudError {
      widget.app.postNotice(l10n.shareInviteFailed, error: true);
    }
  }

  /// Join by a friend's invite, as if its link had been pasted.
  void _joinFromFriend(FriendInvite invite) {
    _joinInvite.text = invite.text.link;
    _password.text = invite.text.password ?? '';
    _fromFriend = (id: invite.id, link: invite.text.link);
    unawaited(_join());
  }

  /// Copying needs no look at the link, so it never asks.
  void _copy({bool look = false}) {
    Clipboard.setData(ClipboardData(
        text: look
            ? widget.app.share.online?.lookLink ?? ''
            : _link ?? ''));
    widget.app.postNotice(l10n.shareLinkCopied);
  }

  /// Show a link in this window, once the person has been told what
  /// showing it risks, or hide it again. [look] is the invite that can only
  /// look, which is shown and hidden by itself.
  Future<void> _toggleShown({bool look = false}) async {
    if (look ? _shownLook : _shown) {
      return setState(() => look ? _shownLook = false : _shown = false);
    }
    final agreed = await showLumitModal<bool>(
      context: context,
      id: 'share-show',
      builder: (close) => _ShowLinkQuestion(
          online: widget.app.share.online != null,
          look: look,
          onChoose: close),
    );
    if (agreed == true && mounted) {
      setState(() => look ? _shownLook = true : _shown = true);
    }
  }

  /// The person is done with what the last return dropped. When the engine
  /// kept their own copy from before it, that goes too, so they are asked.
  Future<void> _forgetReturn() async {
    final share = widget.app.share;
    final copies = share.returnCopies.length;
    if (copies > 0) {
      final sure = await showLumitModal<bool>(
        context: context,
        id: 'share-forget-ask',
        builder: (close) => _Question(
          title: l10n.shareReturnForgetTitle(copies),
          body: l10n.shareReturnForgetBody(copies),
          no: l10n.cancel,
          yes: l10n.delete,
          keyPrefix: 'share-forget-ask',
          onChoose: close,
        ),
      );
      if (sure != true) return;
    }
    share.forgetReturn();
  }

  /// Look for a lost host by the invite just pasted.
  void _reinvite() {
    final needed = _needsPassword && _password.text.isEmpty;
    final taken = !needed &&
        widget.app.share
            .reinvite(_newInvite.text.trim(), password: _passwordNow());
    if (taken) {
      _newInvite.clear();
      _password.clear();
    }
    setState(() => _error = taken
        ? null
        : needed
            ? l10n.sharePasswordNeeded
            : l10n.shareBadInvite);
  }

  /// Take a guest out. The engine replaces the invite as it does, so the one
  /// they hold stops working. The new one is shown, and its key is kept for
  /// sharing this project again.
  void _remove(int person) {
    final share = widget.app.share;
    share.remove(person);
    final key = share.key();
    final (id, port) = (_projectId, share.port);
    if (key != null && id != null && port != null) {
      _prefs.setShareHosted(id, '$port/$key');
    }
    setState(() {
      _link = _linkNow();
      _keptKey = key ?? _keptKey;
      _sameInvite = false;
    });
  }

  /// Ask before taking someone out, since it replaces the invite for
  /// everyone, and then do it.
  Future<void> _askRemove(BridgeSharePerson person) async {
    final sure = await showLumitModal<bool>(
      context: context,
      id: 'share-remove-ask',
      builder: (close) => _Question(
        title: l10n.shareRemoveTitle(person.name),
        body: l10n.shareRemoveBody,
        no: l10n.shareRemoveNo,
        yes: l10n.shareRemove,
        keyPrefix: 'share-remove-ask',
        onChoose: close,
      ),
    );
    if (sure != true || !mounted) return;
    if (widget.app.share.online == null) return _remove(person.id);
    await _removeOnline(person.id);
  }

  /// Take someone out of a project kept online. Lumit's server moves it to
  /// a new invite, which is the one this window holds from then on.
  Future<void> _removeOnline(int person) async {
    setState(() {
      _busy = true;
      _removing = person;
      _error = null;
    });
    String? error;
    try {
      await _ui.removeOnline(person);
    } on CloudError catch (e) {
      error = shareOnlineErrorText(e.code);
    }
    if (!mounted) return;
    setState(() {
      _busy = false;
      _removing = null;
      _error = error;
      _newLink = _newLink || error == null;
    });
    _readLink();
  }

  /// Stop keeping the project online, once the host has said they mean it:
  /// nobody else can carry on from there.
  Future<void> _stopOnline() async {
    if (!await askStopKeeping(context) || !mounted) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await _ui.stopOnline();
    } on CloudError catch (e) {
      if (mounted) {
        setState(() {
          _busy = false;
          _error = shareOnlineErrorText(e.code);
        });
      }
      return;
    }
    widget.onClose();
  }

  /// Stop sharing for everyone, or leave. A host's invite is finished with,
  /// so the next time this project is shared it gets a new one.
  void _stop() {
    final id = _projectId;
    final share = widget.app.share;
    if (share.role == ShareRole.host && share.online == null && id != null) {
      _prefs.setShareHosted(id, null);
    }
    widget.app.stopSharing();
    widget.onClose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final share = widget.app.share;
    // Who is here changing is also how this hears sharing start and stop, and
    // the host being lost and found.
    return ListenableBuilder(
      listenable:
          Listenable.merge([share.roster, share.footage, _ui.following]),
      builder: (context, _) => DialogFrame(
        width: shareDialogWidth,
        children: [
          dialogTitleBar(
            t,
            title: l10n.shareTitle,
            onClose: widget.onClose,
            keyPrefix: 'share',
          ),
          if (!share.active)
            dialogTabs<_Page>(
              t,
              tabs: [
                (_Page.share, l10n.shareTabShare),
                (_Page.join, l10n.shareTabJoin),
              ],
              current: _page,
              onPick: (page) => setState(() {
                _page = page;
                _error = null;
              }),
              keyPrefix: 'share',
            ),
          // A project kept online has the most to show, so its body is what
          // scrolls when the screen is short, and then nothing inside it
          // does.
          if (share.online != null)
            _ScrollList(
              maxHeight: math.max(
                  200, MediaQuery.sizeOf(context).height - _aroundBody),
              onScrolls: () {
                if (!_oneScroller) setState(() => _oneScroller = true);
              },
              children: [_body(_onlineRows(t, share))],
            )
          else
            _body(switch ((share.role, _page)) {
              (ShareRole.none, _Page.share) => _startRows(t),
              (ShareRole.none, _Page.join) => _joinRows(t),
              (ShareRole.host, _) => [
                  ..._inviteRows(t, share),
                  _people(t, share),
                  ..._transfers(t, share),
                ],
              (ShareRole.guest, _) => [
                  ..._guestRows(t, share),
                  _people(t, share),
                  ..._transfers(t, share),
                  _advancedFold(t),
                  if (_advanced) ..._footageRows(t),
                ],
            }),
          dialogFooter(
            t,
            keyPrefix: 'share',
            // What ends it stands apart from the way out of the window.
            leading: switch (share.role) {
              ShareRole.none => null,
              // Not while the server is being asked something, or the
              // project is on its way to this Lumit.
              _ when share.online != null && _hosting(share) => _ending(
                  t,
                  'share-stop',
                  l10n.settingsOnlineStop,
                  _busy || share.forwarding ? null : _stopOnline,
                ),
              // Anyone else in a project kept online leaves it, a host on
              // a machine that can only look included.
              ShareRole.host when share.online == null =>
                _ending(t, 'share-stop', l10n.shareStop, _stop),
              _ => _ending(t, 'share-leave', l10n.shareLeave, _stop),
            },
            actions: [
              switch ((share.role, _page)) {
                (ShareRole.none, _Page.share) => HouseButton(
                    key: const ValueKey('share-start'),
                    primary: true,
                    autofocus: true,
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    onPressed:
                        widget.app.project == null || _busy ? null : _start,
                    child: Text(
                        _busy ? l10n.shareOnlineStarting : l10n.shareStart),
                  ),
                (ShareRole.none, _Page.join) => HouseButton(
                    key: const ValueKey('share-join'),
                    primary: true,
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    onPressed: _joining ? null : _join,
                    child: Text(_moving
                        ? l10n.shareForwardMoving
                        : _joining
                            ? l10n.shareJoining
                            : l10n.shareJoin),
                  ),
                _ => HouseButton(
                    key: const ValueKey('share-done'),
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    onPressed: widget.onClose,
                    child: Text(l10n.done),
                  ),
              },
            ],
          ),
        ],
      ),
    );
  }

  /// What ends sharing, at the foot of the window: plain words, so that
  /// Done is the one button there.
  Widget _ending(
          LumitTheme t, String id, String label, VoidCallback? onPressed) =>
      HouseButton(
        key: ValueKey<String>(id),
        frameless: true,
        padding: const EdgeInsets.symmetric(horizontal: 4),
        onPressed: onPressed,
        child: Text(label,
            style: t.small.copyWith(
                color: onPressed == null ? t.textDisabled : t.textMuted)),
      );

  Widget _nameRow(LumitTheme t, VoidCallback onSubmitted) => dialogRow(
        t,
        l10n.shareYourName,
        _field(t, 'share-name', _name,
            hint: l10n.shareDefaultName, onSubmitted: onSubmitted),
        labelColumn: _labelColumn,
      );

  /// The fold the rows most people never need sit under.
  Widget _advancedFold(LumitTheme t) => Padding(
        padding: const EdgeInsets.only(top: 8),
        child: HoverMark(
          builder: (context, lit) => GestureDetector(
            key: const ValueKey('share-advanced'),
            behavior: HitTestBehavior.opaque,
            onTap: () => setState(() => _advanced = !_advanced),
            child: Row(
              children: [
                SizedBox(
                  width: _leadGlyph,
                  child: TwirlTurn(
                    open: _advanced,
                    child: lumitIcon(
                      _advanced ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                      size: 12,
                      color: Color.lerp(t.textMuted, t.textPrimary, lit)!,
                    ),
                  ),
                ),
                const SizedBox(width: _leadGap),
                Text(l10n.shareAdvanced,
                    style: t.small.copyWith(
                        color:
                            Color.lerp(t.textSecondary, t.textPrimary, lit))),
              ],
            ),
          ),
        ),
      );

  /// A link, hidden until the person asks to see it. While it is hidden it
  /// is not drawn, read out or selectable: the well holds only ink, and the
  /// room the link takes up. [shown] is for a link with a Show of its own.
  Widget _linkWell(LumitTheme t, String id, String link, {bool? shown}) =>
      Container(
        padding: const EdgeInsets.all(8),
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.wellRadius),
        ),
        child: _Veil(
          shown: shown ?? _shown,
          lineHeight: (t.mono.fontSize ?? 11) * (t.mono.height ?? 1.3),
          builder: (hidden) => hidden == 0
              ? SelectableText(
                  link,
                  key: ValueKey<String>(id),
                  style: t.mono,
                  selectionColor: t.accent.withValues(alpha: 0.5),
                )
              : ExcludeSemantics(
                  child: IgnorePointer(
                    child: Opacity(
                      opacity: 1 - hidden,
                      child: Text(link,
                          key: ValueKey<String>(id), style: t.mono),
                    ),
                  ),
                ),
        ),
      );

  /// A field whose text is hidden: typed and pasted into as usual, with the
  /// letters not drawn and ink where they would be.
  Widget _veiledField(
    LumitTheme t,
    String id,
    TextEditingController controller, {
    required bool shown,
    String? hint,
    VoidCallback? onSubmitted,
  }) =>
      _Veil(
        shown: shown,
        inset: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        // Dots under the ink, so what is typed is never laid out while it
        // is hidden, not even see-through.
        builder: (hidden) => _field(t, id, controller,
            hint: hint,
            onSubmitted: onSubmitted,
            obscure: !shown,
            style: hidden == 0
                ? null
                : t.bodyPrimary.copyWith(
                    color: t.textPrimary.withValues(alpha: 1 - hidden))),
      );

  /// The password row: the field, hidden like a link once something is in
  /// it, and a button that shows it. A password is the person's own to
  /// look at, so this one does not ask first.
  Widget _passwordRow(LumitTheme t, String hint, VoidCallback onSubmitted) =>
      dialogRow(
        t,
        l10n.sharePassword,
        Row(
          children: [
            Expanded(
              child: _veiledField(t, 'share-password', _password,
                  shown: _passwordShown || _password.text.isEmpty,
                  hint: hint,
                  onSubmitted: onSubmitted),
            ),
            const SizedBox(width: 8),
            HouseButton(
              key: const ValueKey('share-password-show'),
              small: true,
              onPressed: () => setState(() => _passwordShown = !_passwordShown),
              child: Text(
                  _passwordShown ? l10n.shareHideLink : l10n.shareShowLink,
                  style: t.small),
            ),
          ],
        ),
        labelColumn: _labelColumn,
      );

  Widget _showButton(LumitTheme t, {bool look = false}) => HouseButton(
        key: ValueKey<String>(look ? 'share-show-look' : 'share-show'),
        small: true,
        onPressed: () => _toggleShown(look: look),
        child: Text(
            (look ? _shownLook : _shown)
                ? l10n.shareHideLink
                : l10n.shareShowLink,
            style: t.small),
      );

  /// A tick box with what it means beside it.
  Widget _tick(LumitTheme t, String id, String text, bool on,
          ValueChanged<bool> onChanged) =>
      Padding(
        padding: const EdgeInsets.only(bottom: 4),
        child: Row(
          children: [
            HouseCheckbox(
                key: ValueKey<String>(id), value: on, onChanged: onChanged),
            const SizedBox(width: 6),
            Expanded(child: Text(text, style: t.small)),
          ],
        ),
      );

  /// How much smaller the stand-ins this computer makes are, one way of
  /// two: 1, 2 or 4, named in that order. Nothing to choose while it sends
  /// none.
  Widget _smallerRow(LumitTheme t, String label, String id, int value,
          List<String> names, ValueChanged<int> onChanged) =>
      dialogRow(
        t,
        label,
        dialogDropdown<int>(t,
            id: id,
            value: value,
            options: const [1, 2, 4],
            label: (n) => names[n == 1 ? 0 : n == 2 ? 1 : 2],
            onChanged: _prefs.shareGive
                ? (n) {
                    onChanged(n);
                    setState(() {});
                  }
                : null),
        labelColumn: _labelColumn,
      );

  /// A speed limit: a number, and the unit it is in.
  Widget _limitRow(LumitTheme t, String label, String id,
          TextEditingController controller) =>
      dialogRow(
        t,
        label,
        Row(
          children: [
            Expanded(
                child: _field(t, id, controller, hint: l10n.shareLimitNone)),
            const SizedBox(width: 8),
            Text(l10n.shareLimitUnit,
                style: t.small.copyWith(color: t.textMuted)),
          ],
        ),
        labelColumn: _labelColumn,
      );

  /// Footage for whoever has not got it: whether this computer sends any
  /// and asks for any, and how fast. The computer's, not the project's, so
  /// it is the same rows on every page and takes effect as it is changed.
  ///
  /// With [looking] this machine can only look at the project, so it sends
  /// nothing, and the rows about sending are left out.
  List<Widget> _footageRows(LumitTheme t, {bool looking = false}) => [
        const SizedBox(height: 6),
        dialogRow(
          t,
          l10n.shareFootage,
          Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (!looking)
                _tick(t, 'share-give', l10n.shareGive, _prefs.shareGive,
                    (on) {
                  _prefs.setShareFootage(give: on);
                  setState(() {});
                }),
              _tick(t, 'share-take', l10n.shareTake, _prefs.shareTake, (on) {
                _prefs.setShareFootage(take: on);
                setState(() {});
              }),
            ],
          ),
          labelColumn: _labelColumn,
        ),
        if (!looking) ...[
          _smallerRow(
              t,
              l10n.shareStandInSize,
              'share-size',
              _prefs.shareSize,
              [l10n.shareSizeFull, l10n.shareSizeHalf, l10n.shareSizeQuarter],
              (n) => _prefs.setShareFootage(size: n)),
          _smallerRow(
              t,
              l10n.shareStandInFrames,
              'share-every',
              _prefs.shareEvery,
              [
                l10n.shareFramesAll,
                l10n.shareFramesHalf,
                l10n.shareFramesQuarter
              ],
              (n) => _prefs.setShareFootage(every: n)),
          _limitRow(t, l10n.shareUpLimit, 'share-up-limit', _upLimit),
        ],
        _limitRow(t, l10n.shareDownLimit, 'share-down-limit', _downLimit),
        _line(t, l10n.shareLimitHint),
      ];

  /// The footage crossing just now, each way, and how far along it is.
  List<Widget> _transfers(LumitTheme t, ShareState share) {
    final moving = share.transfers();
    if (moving.isEmpty) return const [];
    String line(BridgeShareTransfer transfer) {
      final total = transfer.total.toInt();
      final percent =
          total == 0 ? 0 : (transfer.done.toInt() * 100 / total).round();
      final name = transfer.name.isEmpty ? l10n.shareAnExport : transfer.name;
      return transfer.sending
          ? l10n.shareSending(name, percent)
          : l10n.shareTaking(name, percent);
    }

    return [
      dialogGroup(t, l10n.shareFootage, [
        for (final transfer in moving)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 2),
            child: Text(line(transfer),
                style: t.small, overflow: TextOverflow.ellipsis),
          ),
      ]),
    ];
  }

  /// Sharing the open project: who this person is, and under the fold where
  /// to listen and how people far away get in.
  List<Widget> _startRows(LumitTheme t) => [
        _nameRow(t, _start),
        _line(t, l10n.shareHostHint),
        _passwordRow(
            t,
            _keptPassword ? l10n.sharePasswordKept : l10n.sharePasswordNone,
            _start),
        _line(
            t,
            _online && _ui.account.pro
                ? l10n.sharePasswordHintOnline
                : l10n.sharePasswordHint),
        _cloudRow(t),
        _onlineRow(t),
        _advancedFold(t),
        if (_advanced) ...[
          const SizedBox(height: 4),
          dialogRow(
            t,
            l10n.shareOutside,
            Row(
              children: [
                HouseCheckbox(
                  key: const ValueKey('share-outside'),
                  value: _outside,
                  onChanged: (on) {
                    _prefs.setShareOutside(on);
                    setState(() => _outside = on);
                  },
                ),
                const SizedBox(width: 6),
                Expanded(child: Text(l10n.shareOutsideAsk, style: t.small)),
              ],
            ),
            labelColumn: _labelColumn,
          ),
          dialogRow(
            t,
            l10n.sharePort,
            _field(t, 'share-port', _port, onSubmitted: _start),
            labelColumn: _labelColumn,
          ),
          dialogRow(
            t,
            l10n.shareRelay,
            _field(t, 'share-relay', _relay,
                hint: l10n.shareRelayExample, onSubmitted: _start),
            labelColumn: _labelColumn,
          ),
          _line(t, l10n.shareRelayHint),
          ..._footageRows(t),
        ],
        if (_error case final error?) _line(t, error, warning: true),
      ];

  /// One of Lumit Pro's two ways to share: a tick for an account with Pro,
  /// and for anyone else what Pro would do here and the way to it. [off] is
  /// a row the other tick has left with no part to play.
  Widget _proRow(LumitTheme t, String id, String label,
          {required String use,
          required String pro,
          required bool value,
          required ValueChanged<bool> onChanged,
          bool off = false}) =>
      ListenableBuilder(
        listenable: _ui.account,
        builder: (context, _) => dialogRow(
          t,
          label,
          _ui.account.pro
              ? IgnorePointer(
                  ignoring: off,
                  child: Opacity(
                    opacity: off ? 0.4 : 1,
                    child: Row(
                      children: [
                        HouseCheckbox(
                          key: ValueKey<String>(id),
                          value: value && !off,
                          onChanged: onChanged,
                        ),
                        const SizedBox(width: 6),
                        Expanded(child: Text(use, style: t.small)),
                      ],
                    ),
                  ),
                )
              : Row(
                  children: [
                    Expanded(
                      child: Text(pro,
                          style: t.small.copyWith(color: t.textMuted)),
                    ),
                    const SizedBox(width: 8),
                    HouseButton(
                      key: ValueKey<String>('$id-pro'),
                      small: true,
                      onPressed: () => unawaited(showProWindow(context)),
                      child: Text(l10n.shareCloudSee, style: t.small),
                    ),
                  ],
                ),
          labelColumn: _labelColumn,
          dimmed: off && _ui.account.pro,
        ),
      );

  /// Lumit's own relay, which a project kept online has no use for.
  Widget _cloudRow(LumitTheme t) => _proRow(t, 'share-cloud', l10n.shareCloud,
      use: l10n.shareCloudUse,
      pro: l10n.shareCloudPro,
      value: _prefs.shareCloud,
      onChanged: (on) => setState(() => _prefs.setShareCloud(on)),
      off: _online);

  /// Keeping the project online, which is the other way to share it. Ticked,
  /// it is what the one button at the foot does.
  Widget _onlineRow(LumitTheme t) =>
      _proRow(t, 'share-online', l10n.shareOnline,
          use: l10n.shareOnlineUse,
          pro: l10n.shareOnlinePro,
          value: _online,
          onChanged: (on) => setState(() => _online = on));

  /// Joining somebody else's: their link, and where the footage is here.
  List<Widget> _joinRows(LumitTheme t) => [
        for (final invite in _ui.friendInvites.waiting)
          dialogRow(
            t,
            l10n.shareFriendInviteFrom(invite.from.name),
            Row(
              children: [
                Expanded(
                  child: Text(
                      invite.text.look
                          ? l10n.shareFriendInviteLook(invite.text.project)
                          : invite.text.project,
                      overflow: TextOverflow.ellipsis),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: ValueKey<String>('share-friend-decline-${invite.id}'),
                  small: true,
                  onPressed: () =>
                      unawaited(_ui.friendInvites.dismiss(invite.id)),
                  child: Text(l10n.shareFriendInviteDecline, style: t.small),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: ValueKey<String>('share-friend-join-${invite.id}'),
                  small: true,
                  primary: true,
                  onPressed: _joining ? null : () => _joinFromFriend(invite),
                  child: Text(l10n.shareJoin),
                ),
              ],
            ),
            labelColumn: _labelColumn,
          ),
        _nameRow(t, _join),
        dialogRow(
          t,
          l10n.shareLink,
          Row(
            children: [
              Expanded(
                // What was pasted is hidden like any other link, and still
                // takes a paste over it.
                child: _veiledField(t, 'share-join-invite', _joinInvite,
                    shown: _shown || _joinInvite.text.isEmpty,
                    hint: l10n.shareLinkPaste,
                    onSubmitted: _join),
              ),
              const SizedBox(width: 8),
              _showButton(t),
            ],
          ),
          labelColumn: _labelColumn,
        ),
        // What came of the link, under the link.
        ...switch (_joinMet) {
          // Waiting on the host is not something wrong with the link.
          BridgeShareVersions_HostFirst() => [
              _line(t, l10n.shareVersionsHostFirstJoin(shareThisLumit())),
            ],
          BridgeShareVersions_Update(:final version) => [
              _actionLine(
                t,
                l10n.shareVersionsUpdateJoin(version),
                _updateButton(t),
              ),
            ],
          _ => [
              if (_error case final error?) _line(t, error, warning: true),
            ],
        },
        if (_needsPassword) _passwordRow(t, l10n.sharePasswordTheirs, _join),
        dialogRow(
          t,
          l10n.shareFootageFolder,
          Row(
            children: [
              Expanded(
                child: Text(_footage ?? '',
                    style: t.small, overflow: TextOverflow.ellipsis),
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('share-join-footage'),
                small: true,
                onPressed: _pickFootage,
                child: Text(l10n.chooseEllipsis, style: t.small),
              ),
            ],
          ),
          labelColumn: _labelColumn,
        ),
        _advancedFold(t),
        if (_advanced) ..._footageRows(t),
      ];

  /// A sentence about what is in the way, with the one thing to press about
  /// it at its end.
  Widget _actionLine(LumitTheme t, String text, Widget? action,
          {bool wrong = true}) =>
      Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Row(
          children: [
            Expanded(
              child: Text(text,
                  style: t.small.copyWith(
                      color: wrong ? t.warning : t.textSecondary)),
            ),
            if (action != null) ...[const SizedBox(width: 8), action],
          ],
        ),
      );

  /// The way to a newer Lumit, saying what the Help menu's own row says.
  Widget _updateButton(LumitTheme t) => ListenableBuilder(
        listenable: _ui.updates,
        builder: (context, _) => HouseButton(
          key: const ValueKey('share-update'),
          small: true,
          onPressed: _ui.updates.busy
              ? null
              : () => unawaited(offerLumitUpdate(context, widget.app, _ui)),
          child: Text(_ui.updates.menuLabel, style: t.small),
        ),
      );

  /// Who the link works for, in a sentence: the router's answer and the
  /// relay's, read together.
  Widget _reachLine(LumitTheme t, ShareState share) {
    final relay = share.relayed;
    final open = share.reach is BridgeShareReach_Open;
    if (open || relay == BridgeShareRelayed.open) {
      return _line(t, open ? l10n.shareOpenAnywhere : l10n.shareOpenRelay);
    }
    if (share.reach is BridgeShareReach_Asking ||
        relay == BridgeShareRelayed.asking) {
      return _line(t, l10n.shareOpenChecking);
    }
    if (relay == BridgeShareRelayed.unreachable) {
      return _line(t, l10n.shareRelayDown, warning: true);
    }
    return _line(
        t,
        share.reach is BridgeShareReach_Off
            ? l10n.shareOpenLocalOff
            : l10n.shareOpenLocal,
        warning: true);
  }

  /// While hosting: the link to send, and who it works for.
  List<Widget> _inviteRows(LumitTheme t, ShareState share) => [
        dialogRow(
          t,
          l10n.shareLink,
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              _showButton(t),
              const SizedBox(width: 8),
              Builder(
                builder: (button) => HouseButton(
                  key: const ValueKey('share-send-friend'),
                  small: true,
                  onPressed: () => unawaited(_pickFriend(button)),
                  child: Text(l10n.shareSendToFriend, style: t.small),
                ),
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('share-copy'),
                small: true,
                primary: true,
                onPressed: _copy,
                child: Text(l10n.shareCopyLink),
              ),
            ],
          ),
          labelColumn: _labelColumn,
        ),
        // On a line of its own: a link is far longer than a row's control.
        _linkWell(t, 'share-invite', _link ?? ''),
        if (_sameInvite) _line(t, l10n.shareSameInvite),
        if (_keptPassword) _line(t, l10n.sharePasswordSet),
        _reachLine(t, share),
        _advancedFold(t),
        if (_advanced) ...[
          const SizedBox(height: 4),
          dialogRow(
            t,
            l10n.shareAddress,
            _field(t, 'share-address', _address),
            labelColumn: _labelColumn,
          ),
          _line(t, l10n.shareAddressHint),
          ..._footageRows(t),
        ],
        const SizedBox(height: dialogGroupGap),
      ];

  /// While a guest: what wants doing about a host out of reach, and about
  /// conflicts a merge left.
  List<Widget> _guestRows(LumitTheme t, ShareState share) => [
        if (share.away) ...[
          Text(l10n.shareAway, style: t.small.copyWith(color: t.warning)),
          const SizedBox(height: 6),
          ..._reinviteRows(t),
          if (_error case final error?) _line(t, error, warning: true),
        ],
        if (share.held > 0) _conflictsRow(t, share),
        if (share.dropped > 0 || share.returnCopies.isNotEmpty)
          _ScrollList(
            maxHeight: _activityListHeight,
            gutter: 8,
            children: [_droppedRow(t, share)],
          ),
        if (share.away ||
            share.held > 0 ||
            share.dropped > 0 ||
            share.returnCopies.isNotEmpty)
          const SizedBox(height: dialogGroupGap),
      ];

  /// What coming back cost this person, at the head of who changed what.
  /// One block for each copy the engine kept of their own project from just
  /// before a return, the latest first, each saying what its own return
  /// dropped. They stay until dismissed, and are here again when the copy
  /// is opened in another run of Lumit.
  Widget _droppedRow(LumitTheme t, ShareState share) {
    final copies = share.returnCopies;
    final why = copies.indexWhere((copy) => copy.refused > 0);
    return Column(
      key: const ValueKey('share-dropped'),
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // A return whose copy could not be written is known only by what
        // the engine said of it as it happened.
        if (copies.isEmpty)
          _returnRow(t, refused: share.dropped, what: share.droppedWhat)
        else
          for (final (index, copy) in copies.indexed)
            _returnRow(t,
                refused: copy.refused,
                what: copy.dropped,
                copy: copy,
                index: index,
                why: index == why,
                several: copies.length > 1),
      ],
    );
  }

  /// What one return cost: how many of this person's changes it dropped,
  /// what each of the first twenty was and was to, and the way to their own
  /// project from just before it ([copy]). It wears the mark the window's
  /// first line wears in trouble, since a dot there is a person. With
  /// nothing dropped it is only the way back, which the engine keeps when
  /// conflicts were held too. [why] says once why changes are dropped, and
  /// with [several] the copies are told apart by when each was written.
  Widget _returnRow(LumitTheme t,
      {required int refused,
      required List<BridgeShareDropped> what,
      BridgeShareReturnCopy? copy,
      int index = 0,
      bool why = true,
      bool several = false}) {
    final quiet = t.small.copyWith(color: t.textMuted);
    final lost = refused > 0;
    final earlier = index > 0;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: personDiscSize + 8,
            child: Align(
              alignment: Alignment.centerLeft,
              child: lumitIcon(LumitIcon.link,
                  size: personDiscSize,
                  color: lost ? t.warning : t.textMuted),
            ),
          ),
          Expanded(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                    lost
                        ? l10n.shareDropped(refused)
                        : l10n.shareReturnCopyKept(1),
                    style: lost ? t.body.copyWith(color: t.warning) : t.body,
                    overflow: TextOverflow.ellipsis),
                if (lost && why)
                  Padding(
                    padding: const EdgeInsets.only(top: 1, bottom: 2),
                    child: Text(l10n.shareDroppedWhy, style: quiet),
                  ),
                // Each as the step it was, then what it was to.
                for (final (at, one) in what.indexed)
                  Text.rich(
                    TextSpan(
                      text: engineLabel(one.step),
                      style: t.small.copyWith(color: t.textSecondary),
                      children: [
                        if (one.item != null || one.layer != null)
                          TextSpan(
                              text:
                                  '  ${[one.item, one.layer].nonNulls.join(' › ')}',
                              style: quiet),
                      ],
                    ),
                    key: ValueKey<String>(
                        'share-dropped-${earlier ? 'earlier-' : ''}$at'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                if (what.isNotEmpty && refused > what.length)
                  Text(l10n.shareActivityMore(refused - what.length),
                      style: quiet),
                if (copy != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 5, bottom: 2),
                    child: HouseButton(
                      key: ValueKey<String>(earlier
                          ? 'share-open-copy-earlier'
                          : 'share-open-copy'),
                      small: true,
                      onPressed: () => unawaited(openReturnCopy(
                          context, widget.app, copy, widget.onClose,
                          picker: widget.savePicker)),
                      child: Text(
                          several
                              ? l10n.shareOpenMyCopyFrom(
                                  _copyWhen(_copyAt(copy)))
                              : l10n.shareOpenMyCopy,
                          style: t.small),
                    ),
                  ),
              ],
            ),
          ),
          // One way to be done with them all, on the first.
          if (earlier)
            const SizedBox(width: dialogCloseGlyph + 10)
          else
            _glyphButton(t, 'share-dropped-seen', LumitIcons.close,
                l10n.close, () => unawaited(_forgetReturn())),
        ],
      ),
    );
  }

  /// When a copy was written, to tell two apart: the time, with the day
  /// before it once that is not today.
  String _copyWhen(DateTime at) {
    final now = DateTime.now();
    final today =
        at.year == now.year && at.month == now.month && at.day == now.day;
    return today
        ? backupTime(at)
        : l10n.backupDayTime(writtenDay(at), backupTime(at));
  }

  /// A small glyph to press at the end of a row, with what it does on hover.
  Widget _glyphButton(LumitTheme t, String id, String mark, String says,
          VoidCallback onTap,
          {Color? colour}) =>
      LumitTooltip(
        message: says,
        child: MouseRegion(
          cursor: SystemMouseCursors.click,
          child: GestureDetector(
            key: ValueKey<String>(id),
            behavior: HitTestBehavior.opaque,
            onTap: onTap,
            child: SizedBox(
              width: dialogCloseGlyph + 10,
              height: _listRow - 2,
              child: Align(
                alignment: Alignment.centerRight,
                child: glyph.LumitIcon(
                  mark,
                  size: dialogCloseGlyph,
                  colour: colour ?? t.textMuted,
                  semanticLabel: says,
                ),
              ),
            ),
          ),
        ),
      );

  /// Where someone away pastes a new invite to look by. `show` is off where
  /// the window already has the button that unveils it.
  List<Widget> _reinviteRows(LumitTheme t, {bool show = true}) => [
        dialogRow(
          t,
          l10n.shareNewInvite,
          Row(
            children: [
              Expanded(
                // A pasted invite is a link like the one joined by, and is
                // hidden the same way.
                child: _veiledField(t, 'share-reinvite', _newInvite,
                    shown: _shown || _newInvite.text.isEmpty,
                    onSubmitted: _reinvite),
              ),
              const SizedBox(width: 8),
              if (show) ...[
                _showButton(t),
                const SizedBox(width: 8),
              ],
              HouseButton(
                key: const ValueKey('share-reconnect'),
                small: true,
                onPressed: _reinvite,
                child: Text(l10n.shareReconnect, style: t.small),
              ),
            ],
          ),
          labelColumn: _labelColumn,
        ),
        if (_needsPassword) _passwordRow(t, l10n.sharePasswordTheirs, _reinvite),
      ];

  /// How many conflicts a merge left, and the way to them.
  Widget _conflictsRow(LumitTheme t, ShareState share) => dialogRow(
        t,
        l10n.shareConflictsTitle,
        Row(
          children: [
            Expanded(
              child:
                  Text(l10n.shareConflictsHeld(share.held), style: t.small),
            ),
            HouseButton(
              key: const ValueKey('share-review'),
              small: true,
              onPressed: () => showShareConflictsFrb(context, widget.app),
              child: Text(l10n.shareReview, style: t.small),
            ),
          ],
        ),
        labelColumn: _labelColumn,
      );

  /// While the project is kept online: that it is, the link to send, how
  /// this machine stands with Lumit's server, who is here and what footage
  /// is crossing.
  List<Widget> _onlineRows(LumitTheme t, ShareState share) {
    final hosting = _hosting(share);
    final trouble = shareTrouble(share);
    final link = _link ?? '';
    final looking = share.online?.look ?? false;
    final lookLink = share.online?.lookLink ?? '';
    final lookOn = _lookAsked ?? share.online?.lookOn ?? false;
    // A guest away from the server with nothing else in the way.
    final lost =
        share.away && share.writable && share.versions == null && !hosting;
    return [
      // How it stands comes first: that it is kept online, and opposite it
      // whether this machine is in step.
      Row(
        children: [
          SizedBox(
            width: _leadGlyph,
            // The sweep is Lumit Pro at work, so it goes when something is
            // in the way of that.
            child: trouble == null
                ? proSwept(lumitIcon(LumitIcon.link,
                    size: _leadGlyph, color: t.textPrimary))
                : lumitIcon(LumitIcon.link,
                    size: _leadGlyph,
                    color: trouble.wrong ? t.warning : t.textMuted),
          ),
          const SizedBox(width: _leadGap),
          Expanded(
            child: Text(l10n.shareOnlineLine,
                key: const ValueKey('share-online-line'),
                style: t.small.copyWith(color: t.textSecondary),
                overflow: TextOverflow.ellipsis),
          ),
          const SizedBox(width: 8),
          if (trouble == null && looking)
            Text(l10n.shareLookOnly,
                key: const ValueKey('share-online-look'),
                style: t.small.copyWith(
                    color: t.textPrimary, fontWeight: FontWeight.w600))
          else if (trouble == null) ...[
            SizedBox(
              width: _leadGlyph,
              child:
                  glyph.LumitIcon(LumitIcons.tick, size: 12, colour: t.success),
            ),
            const SizedBox(width: _leadGap),
            Text(l10n.shareOnlineInStep,
                key: const ValueKey('share-online-in-step'),
                style: t.small.copyWith(color: t.textSecondary)),
          ] else
            Text(trouble.words,
                key: const ValueKey('share-online-trouble'),
                style: t.small.copyWith(
                    color: trouble.wrong ? t.warning : t.textSecondary)),
        ],
      ),
      // What is in the way, under the word for it: quiet, never the word
      // again, and with the one thing to do about it at its end.
      if (trouble != null && !share.forwarding)
        _actionLine(
          t,
          trouble.line,
          switch (share.versions) {
            // Asked again here, since it cannot be taken back. Not on a
            // machine that can only look: the server would refuse it.
            BridgeShareVersions_Movable() when !hosting => null,
            BridgeShareVersions_Movable(:final version) => HouseButton(
                key: const ValueKey('share-forward'),
                small: true,
                onPressed: () async {
                  if (await askForward(context, version) && mounted) {
                    unawaited(widget.app.moveForward());
                  }
                },
                child: Text(l10n.shareForwardYes, style: t.small),
              ),
            BridgeShareVersions_Update() => _updateButton(t),
            BridgeShareVersions_HostFirst() => null,
            null
                when !share.writable &&
                    hosting &&
                    !(share.online?.tooLarge ?? false) =>
              HouseButton(
                key: const ValueKey('share-online-pro'),
                small: true,
                onPressed: () => unawaited(showProWindow(context)),
                child: Text(l10n.shareCloudSee, style: t.small),
              ),
            // Only the host makes a new invite, so only the others can
            // need one, and seldom: nothing is wrong with an invite
            // because a line is down.
            null when lost && !_reinviteOpen && !share.replaced => MouseRegion(
                cursor: SystemMouseCursors.click,
                child: GestureDetector(
                  key: const ValueKey('share-have-invite'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => setState(() => _reinviteOpen = true),
                  child: Text(l10n.shareHaveInvite,
                      style: t.small.copyWith(color: t.textMuted)),
                ),
              ),
            null => null,
          },
          wrong: false,
        ),
      // Where the new invite goes: asked for, or needed at once because
      // the old one was replaced.
      if (lost && (_reinviteOpen || share.replaced)) ...[
        const SizedBox(height: 6),
        ..._reinviteRows(t, show: false),
      ],
      if (_error case final error?) _line(t, error, warning: true),
      const SizedBox(height: 4),
      // The invite that edits, and under it the one that can only look. A
      // machine that can only look holds only the second.
      // A password is the first one's alone, which a lock on its row says.
      // Neither is offered once the host has replaced them: what this copy
      // holds opens nothing.
      if (!looking && !share.replaced)
        ..._onlineLink(t, share,
            look: false,
            label: lookLink.isEmpty ? l10n.shareLink : l10n.shareLinkEdit,
            link: link,
            locked: _linkLocked,
            half: lookOn && lookLink.isNotEmpty,
            lockRoom: _linkLocked),
      if (lookLink.isNotEmpty && !share.replaced)
        ..._onlineLink(t, share,
            look: true,
            label: l10n.shareLinkLook,
            link: lookLink,
            // Someone looking is in by it.
            on: looking || lookOn,
            turns: hosting,
            lockRoom: _linkLocked && !looking),
      if (_newLink) _line(t, l10n.shareOnlineNewLink),
      if (share.held > 0) ...[
        const SizedBox(height: 6),
        _conflictsRow(t, share),
      ],
      const SizedBox(height: dialogGroupGap),
      _people(t, share),
      ListenableBuilder(
        listenable: share.activityRead,
        builder: (context, _) => _activity(t, share),
      ),
      ListenableBuilder(
        listenable: Listenable.merge([_ui.onlineFootage, _ui.backup]),
        builder: (context, _) => _onlineFootage(t, hosting),
      ),
      _advancedFold(t),
      if (_advanced) ..._footageRows(t, looking: looking),
    ];
  }

  /// One invite of a project kept online on one line, veiled as a pasted
  /// one is: the one that edits, or with [look] the one that can only look.
  /// Its whole length is only drawn when it is asked for.
  ///
  /// With [lockRoom] a password is set, and the row says how it stands to
  /// it: a lock for the invite that needs it as well ([locked]), and a tag
  /// for the one that needs none, which nobody should have to read off a
  /// missing lock.
  ///
  /// The invite that can only look is off until the host turns it on, and
  /// [on] says which it is. Off, it opens nothing, so there is no link to
  /// show or copy. With [turns] this machine is the host's and has the
  /// switch. Everyone else reads which it is. While it is on, the invite
  /// that edits shows the project without its password, and its lock is on
  /// changing alone ([half]), which a tag beside the lock says.
  List<Widget> _onlineLink(LumitTheme t, ShareState share,
      {required bool look,
      required String label,
      required String link,
      bool on = true,
      bool turns = false,
      bool locked = false,
      bool half = false,
      bool lockRoom = false}) {
    final shown = look ? _shownLook : _shown;
    final id = look ? 'share-invite-look' : 'share-invite';
    // Copying the invite that edits is the thing to do while nobody else
    // is here, and an ordinary button once someone is.
    final lead = !look && share.others.isEmpty;
    // A password is set and this invite needs none.
    final open = lockRoom && !locked;
    final turn = HouseToggle(
        key: const ValueKey('share-look-on'), value: on, onChanged: _turnLook);
    if (!on) {
      return [
        dialogRow(
          t,
          label,
          Row(children: [turns ? turn : _tag(t, l10n.off)]),
          labelColumn: _labelColumn,
        ),
      ];
    }
    return [
      dialogRow(
        t,
        label,
        Row(
          children: [
            if (turns) ...[turn, const SizedBox(width: 8)],
            Expanded(
              // An invite that needs no password says so where its veil
              // would be: with the switch on the row there is not room for
              // both, and the buttons already say there is a link.
              child: shown || open
                  ? Align(
                      alignment: Alignment.centerLeft,
                      child: open ? _tag(t, l10n.shareLinkNoPassword) : null)
                  : Container(
                      key: ValueKey<String>('$id-veiled'),
                      height: dialogControlHeight,
                      decoration: BoxDecoration(
                        color: t.surface0,
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                      ),
                      child: Row(
                        children: [
                          const SizedBox(width: 7),
                          lumitIcon(LumitIcon.eyeClosed,
                              size: 12, color: t.textMuted),
                          Expanded(
                            child: _Veil(
                              shown: false,
                              inset: const EdgeInsets.fromLTRB(6, 4, 8, 4),
                              builder: (_) => const SizedBox.expand(),
                            ),
                          ),
                        ],
                      ),
                    ),
            ),
            if (locked)
              Padding(
                padding: const EdgeInsets.only(left: 6),
                child: LumitTooltip(
                  key: const ValueKey('share-invite-locked'),
                  message:
                      half ? l10n.sharePasswordSetEdit : l10n.sharePasswordSet,
                  child:
                      lumitIcon(LumitIcon.lock, size: 12, color: t.textSecondary),
                ),
              ),
            if (locked && half)
              Padding(
                padding: const EdgeInsets.only(left: 4),
                child: _tag(t, l10n.shareLinkLockEdit),
              ),
            const SizedBox(width: 8),
            _showButton(t, look: look),
            const SizedBox(width: 8),
            Builder(
              builder: (button) => HouseButton(
                key: ValueKey<String>(
                    look ? 'share-send-friend-look' : 'share-send-friend'),
                small: true,
                onPressed: () => unawaited(_pickFriend(button, look: look)),
                child: Text(l10n.shareSendToFriend, style: t.small),
              ),
            ),
            const SizedBox(width: 8),
            HouseButton(
              key: ValueKey<String>(look ? 'share-copy-look' : 'share-copy'),
              small: true,
              primary: lead,
              onPressed: () => _copy(look: look),
              child: Text(l10n.shareCopyLink, style: lead ? null : t.small),
            ),
          ],
        ),
        labelColumn: _labelColumn,
      ),
      if (shown) _linkWell(t, id, link, shown: shown),
    ];
  }

  /// Who changed what and the notes people left, newest first: what a
  /// person who was not there reads to catch up. Over it, where a note is
  /// written.
  Widget _activity(LumitTheme t, ShareState share) {
    final all = _entries;
    final returned = share.dropped > 0 || share.returnCopies.isNotEmpty;
    // Someone who can only look reads the notes and leaves none.
    final writes = !(share.online?.look ?? false);
    final write = Padding(
      padding: const EdgeInsets.only(bottom: 4),
      child: Row(
        children: [
          Expanded(
            child: _field(t, 'share-note', _note,
                hint: l10n.shareNoteHint, onSubmitted: _sendNote),
          ),
          const SizedBox(width: 8),
          HouseButton(
            key: const ValueKey('share-note-send'),
            small: true,
            onPressed: _sendNote,
            child: Text(l10n.shareNoteSend, style: t.small),
          ),
        ],
      ),
    );
    if (all.isEmpty && !returned) {
      return writes
          ? dialogGroup(t, l10n.shareActivity, [write], key: _activityKey)
          : const SizedBox.shrink();
    }
    final shown = _shownEntries;
    // What is new to this person first, over one rule, and under it what
    // they have seen. A sitting that is new can be older than one of this
    // person's own, which never is, so neither run is all earlier or all
    // later than the other, and the legend does not say that it is.
    final fresh = [
      for (final entry in shown)
        if (_fresh.contains(_entryKey(entry))) entry,
    ];
    final earlier = [
      for (final entry in shown)
        if (!_fresh.contains(_entryKey(entry))) entry,
    ];
    Widget row(Object entry) => switch (entry) {
          BridgeShareNote() => _noteRow(t, share, entry),
          BridgeShareSitting() => _sittingRow(t, share, entry),
          _ => const SizedBox.shrink(),
        };
    return dialogGroup(t, l10n.shareActivity, [
      if (writes) write,
      _ScrollList(
        maxHeight: _activityListHeight,
        free: _oneScroller,
        gutter: 8,
        children: [
          if (returned) _droppedRow(t, share),
          for (final entry in fresh) row(entry),
          if (fresh.isNotEmpty)
            Container(
              key: const ValueKey('share-activity-new'),
              height: 1,
              margin: const EdgeInsets.only(top: 6, bottom: 4),
              decoration: const BoxDecoration(gradient: proSweep),
            ),
          if (fresh.isNotEmpty && earlier.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 2, bottom: 2),
              child: Text(t.kickerCase(l10n.shareActivitySeen),
                  style: t.kicker.copyWith(color: t.textMuted)),
            ),
          for (final entry in earlier) row(entry),
        ],
      ),
      // Outside the list, so it is there to press without scrolling to it.
      if (shown.length < all.length)
        Align(
          alignment: Alignment.centerLeft,
          child: MouseRegion(
            cursor: SystemMouseCursors.click,
            child: GestureDetector(
              key: const ValueKey('share-activity-all'),
              behavior: HitTestBehavior.opaque,
              onTap: () => setState(() {
                _allActivity = true;
                _readNoteWhere();
              }),
              child: Padding(
                padding: const EdgeInsets.only(top: 4, bottom: 2),
                child: Text(l10n.shareActivityAll,
                    style: t.small.copyWith(color: t.textMuted)),
              ),
            ),
          ),
        ),
    ], key: _activityKey);
  }

  /// Who and when, at the head of a sitting or a note. This machine's own
  /// are drawn more quietly. A note still [waiting] to be sent says so where its time
  /// would be.
  Widget _whoRow(LumitTheme t, ShareState share,
      {required String name,
      required bool mine,
      required int at,
      bool waiting = false,
      Widget? action}) {
    final when = DateTime.fromMillisecondsSinceEpoch(at * 1000);
    final now = DateTime.now();
    final today =
        when.year == now.year && when.month == now.month && when.day == now.day;
    final quiet = t.small.copyWith(color: t.textMuted);
    return Row(
      children: [
        PersonDisc(
            colour: share.colourOf(name, mine: mine),
            name: name,
            letters: share.lettersOf(name),
            stale: mine),
        const SizedBox(width: 8),
        Expanded(
          child: Row(
            children: [
              Flexible(
                child: Text(name,
                    style: mine ? t.body.copyWith(color: t.textMuted) : t.body,
                    overflow: TextOverflow.ellipsis),
              ),
              if (mine) ...[
                const SizedBox(width: 6),
                Text(l10n.shareYou, style: quiet),
              ],
            ],
          ),
        ),
        const SizedBox(width: 8),
        if (waiting)
          _tag(t, l10n.shareNoteWaiting)
        else
          Text(today ? backupTime(when) : backupDay(when), style: quiet),
        if (action != null) action,
      ],
    );
  }

  /// Take a note down, for everyone. Someone else's is asked about first,
  /// since it cannot be put back.
  Future<void> _takeDown(BridgeShareNote note) async {
    if (!note.mine) {
      final sure = await showLumitModal<bool>(
        context: context,
        id: 'share-note-down-ask',
        builder: (close) => _Question(
          title: l10n.shareNoteDownTitle(note.name),
          body: l10n.shareNoteDownBody,
          no: l10n.cancel,
          yes: l10n.shareNoteTakeDown,
          keyPrefix: 'share-note-down-ask',
          onChoose: close,
        ),
      );
      if (sure != true) return;
    }
    widget.app.takeNoteDown(note.id);
  }

  /// One note: who and when, as a sitting says them, then its words, and
  /// under them where it was written. A press on that goes there.
  Widget _noteRow(LumitTheme t, ShareState share, BridgeShareNote note) {
    final quiet = t.small.copyWith(color: t.textMuted);
    final where = _noteWhere[note.id];
    final comp = note.comp;
    final place = comp == null || comp.name.isEmpty
        ? null
        : Text.rich(
            TextSpan(
              text: comp.name,
              children: [
                if (where?.timecode case final timecode?) ...[
                  TextSpan(text: '  ·  ', style: quiet),
                  TextSpan(
                      text: timecode,
                      style: t.mono.copyWith(fontSize: dialogMonoSize)),
                ],
                if (where?.layer case final layer?) ...[
                  TextSpan(text: '  ·  ', style: quiet),
                  TextSpan(text: layer),
                ],
              ],
            ),
            style: t.small
                .copyWith(color: note.mine ? t.textMuted : t.textSecondary),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          );
    return Padding(
      key: ValueKey<String>('share-note-${note.id}'),
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _whoRow(t, share,
              name: note.name,
              mine: note.mine,
              at: note.at,
              waiting: note.waiting,
              // Its writer may take it down, and the host anyone's.
              action: note.mine || _hosting(share)
                  ? _glyphButton(
                      t,
                      'share-note-down-${note.id}',
                      LumitIcons.close,
                      l10n.shareNoteTakeDown,
                      () => unawaited(_takeDown(note)))
                  : null),
          Padding(
            padding: const EdgeInsets.only(left: personDiscSize + 8, top: 1),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(note.text,
                    style: t.small.copyWith(
                        color: note.mine ? t.textSecondary : t.textPrimary)),
                if (place != null && comp?.comp == null)
                  place
                else if (place != null)
                  MouseRegion(
                    cursor: SystemMouseCursors.click,
                    child: GestureDetector(
                      key: ValueKey<String>('share-note-go-${note.id}'),
                      behavior: HitTestBehavior.opaque,
                      onTap: () => _goToNote(note),
                      child: place,
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// One sitting: who and when, then how many edits and in which
  /// compositions. A composition's name opens it.
  Widget _sittingRow(
      LumitTheme t, ShareState share, BridgeShareSitting sitting) {
    final quiet = t.small.copyWith(color: t.textMuted);
    final comps = [
      for (final comp in sitting.comps)
        if (comp.name.isNotEmpty) comp,
    ];
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _whoRow(t, share,
              name: sitting.name, mine: sitting.mine, at: sitting.ended),
          Padding(
            padding: const EdgeInsets.only(left: personDiscSize + 8, top: 1),
            child: Wrap(
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                Text(l10n.shareActivityEdits(sitting.edits), style: quiet),
                for (final comp in comps.take(3)) ...[
                  Text('  ·  ', style: quiet),
                  _compName(t, comp, quiet: sitting.mine),
                ],
                if (comps.length > 3)
                  Text(' ${l10n.shareActivityMore(comps.length - 3)}',
                      style: quiet),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// A composition's name, which opens it when it is still in the project.
  Widget _compName(LumitTheme t, BridgeSittingComp comp,
      {required bool quiet}) {
    final name = Text(comp.name,
        style: t.small
            .copyWith(color: quiet ? t.textMuted : t.textSecondary));
    final open = comp.comp;
    if (open == null) return name;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: ValueKey<String>('share-activity-comp-${comp.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () {
          _ui.setSelectedComp(open);
          widget.onClose();
        },
        child: name,
      ),
    );
  }

  /// The footage of a project kept online: the host's choice to keep the
  /// originals there too, and every file on its way up or down.
  Widget _onlineFootage(LumitTheme t, bool hosting) {
    final footage = _ui.onlineFootage;
    final id = _projectId;
    if (!hosting && footage.moves.isEmpty && !footage.full) {
      return const SizedBox.shrink();
    }
    final keeping = id != null && _prefs.shareOriginals.contains(id);
    final backup = _ui.backup;
    final group = dialogGroup(t, l10n.shareFootage, [
      if (hosting && id != null)
        ConstrainedBox(
          constraints: const BoxConstraints(minHeight: _listRow),
          child: Row(
            children: [
              HouseCheckbox(
                key: const ValueKey('share-originals'),
                value: keeping,
                onChanged: (on) {
                  _prefs.setShareOriginals(id, on);
                  footage.look();
                  setState(() {});
                },
              ),
              const SizedBox(width: 6),
              // Unticked, the label says what ticking it would send. Ticked,
              // what the server holds is said beside it.
              Expanded(
                child: Text(
                    !keeping && footage.originalsBytes > 0
                        ? l10n.shareOnlineOriginalsCost(
                            backupSize(footage.originalsBytes))
                        : l10n.shareOnlineOriginals,
                    style: t.small),
              ),
              if (keeping && footage.originalsOnline > 0)
                Text(
                    l10n.shareOnlineOriginalsHeld(
                        backupSize(footage.originalsOnline)),
                    style: t.small.copyWith(color: t.textMuted)),
            ],
          ),
        ),
      if (footage.full)
        ConstrainedBox(
          constraints: const BoxConstraints(minHeight: _listRow),
          child: Row(
            children: [
              Expanded(
                child: Text(
                    !hosting
                        ? l10n.shareOnlineFull
                        : backup.limit > 0
                            ? l10n.shareOnlineFullOwn(backupSize(backup.used),
                                backupSize(backup.limit))
                            : l10n.backupErrorStorageFull,
                    key: const ValueKey('share-online-full'),
                    style: t.small.copyWith(color: t.warning)),
              ),
              // The host is the one who can make room, in Settings.
              if (hosting) ...[
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('share-online-manage'),
                  small: true,
                  onPressed: _manageStorage,
                  child: Text(l10n.shareOnlineManage, style: t.small),
                ),
              ],
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('share-online-full-retry'),
                small: true,
                onPressed: footage.recheck,
                child: Text(l10n.shareOnlineRetry, style: t.small),
              ),
            ],
          ),
        ),
      if (footage.moves.isNotEmpty)
        _ScrollList(
          maxHeight: _footageListHeight,
          free: _oneScroller,
          gutter: 8,
          children: [
            for (final move in footage.moves) _moveRow(t, footage, move),
          ],
        ),
    ]);
    if (footage.moves.isEmpty) return group;
    // How far along it all is, opposite the group's name on its top edge.
    final total = footage.finished + footage.moves.length;
    return Stack(
      children: [
        group,
        Positioned(
          top: 2,
          right: 10,
          child: Container(
            color: t.surface1,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            child: Text(
                l10n.shareOnlineCount(
                    math.min(footage.finished + 1, total), total),
                key: const ValueKey('share-online-count'),
                style: dialogMono(t)),
          ),
        ),
      ],
    );
  }

  /// Settings, where the account's storage and what fills it are. Sending
  /// is tried again on the way back, since room may have been made.
  Future<void> _manageStorage() async {
    await showSettingsWindowFrb(context, initialPage: SettingsPage.account);
    if (!mounted) return;
    _ui.onlineFootage.recheck();
    unawaited(_ui.backup.refresh());
  }

  /// One file on its way: its name, how it is getting on, and a way out.
  Widget _moveRow(LumitTheme t, OnlineFootage footage, OnlineMove move) {
    final percent =
        move.total == 0 ? 0 : (move.done * 100 / move.total).floor();
    final state = switch (move.stage) {
      MoveStage.waiting => move.sending
          ? l10n.shareOnlineWaitingSend
          : l10n.shareOnlineWaitingFetch,
      MoveStage.working when move.total == 0 => l10n.shareOnlinePreparing,
      MoveStage.working => move.sending
          ? l10n.shareOnlineSending(percent)
          : l10n.shareOnlineFetching(percent),
      MoveStage.failed => switch (move.failure) {
          'nothing' => l10n.shareOnlineFailedMake,
          'unreadable' => l10n.shareOnlineFailedOpen,
          _ => move.sending
              ? l10n.shareOnlineFailedSend
              : l10n.shareOnlineFailedFetch,
        },
    };
    final kind = move.original ? 'original' : 'standin';
    final moving = move.stage == MoveStage.working && move.total > 0;
    final failed = move.stage == MoveStage.failed;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          height: _listRow - 2,
          child: Row(
            children: [
              Expanded(
                child: Text(
                    move.original
                        ? l10n.shareOnlineOriginal(move.name)
                        : move.name,
                    style: t.small,
                    overflow: TextOverflow.ellipsis),
              ),
              const SizedBox(width: 8),
              // Which way it is going, so sending and fetching are told
              // apart by more than a word.
              glyph.LumitIcon(
                  move.sending ? LumitIcons.export : LumitIcons.import,
                  size: 12,
                  colour: failed ? t.warning : t.textMuted),
              const SizedBox(width: 4),
              // Figures of one width, so the words keep still as it counts.
              Text(state,
                  style: t.small.copyWith(
                      color: failed ? t.warning : t.textMuted,
                      fontFeatures: const [FontFeature.tabularFigures()])),
              if (failed)
                _glyphButton(
                    t,
                    'share-move-retry-${move.key}-$kind',
                    LumitIcons.reset,
                    l10n.shareOnlineRetry,
                    () => footage.retry(move)),
              _glyphButton(t, 'share-move-cancel-${move.key}-$kind',
                  LumitIcons.close, l10n.cancel, () => footage.cancel(move)),
            ],
          ),
        ),
        // Always its two pixels, so a file starting to move shifts nothing.
        SizedBox(
          height: 2,
          child: moving
              ? HouseProgressBar(
                  fraction: move.done / move.total, height: 2, fill: proSweep)
              : null,
        ),
      ],
    );
  }

  /// Everyone in the project: who is this person, who is the host, where
  /// each is, and for anyone else a way to follow them. The host also has a
  /// way to take a guest out, which shows when a row is pointed at.
  Widget _people(LumitTheme t, ShareState share) {
    final online = share.online != null;
    final hosting = online ? _hosting(share) : share.role == ShareRole.host;
    return dialogGroup(t, l10n.sharePeople, [
      _ScrollList(
        maxHeight: _peopleListHeight,
        free: _oneScroller,
        gutter: 8,
        children: [
          for (final person in share.people)
            _PersonRow(
              key: ValueKey<String>('share-person-${person.id}'),
              person: person,
              letters: share.lettersOf(person.name),
              host: _isHost(share, person),
              look: person.look,
              // Out of step, who is here is only what was last heard, so
              // the others are drawn as not known and without a place.
              stale: share.outOfStep && !person.me,
              comp: _compOf(person),
              onOpenComp: switch (person.comp) {
                final comp? when !person.me => () {
                    _ui.setSelectedComp(comp);
                    widget.onClose();
                  },
                _ => null,
              },
              followed: _ui.following.value == person.id && !person.me,
              onFollow: person.me
                  ? null
                  : () {
                      final followed = _ui.following.value == person.id;
                      _ui.follow(followed ? null : person.id);
                      // The Viewer is what follows, and this window is
                      // over it.
                      if (!followed) widget.onClose();
                    },
              removing: _removing == person.id,
              busy: _busy,
              onRemove: !hosting || person.me || (online && share.away)
                  ? null
                  : () => unawaited(_askRemove(person)),
            ),
          if (share.others.isEmpty)
            SizedBox(
              height: _listRow,
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(l10n.shareNobodyElse,
                    key: const ValueKey('share-nobody-else'),
                    style: t.small.copyWith(color: t.textMuted)),
              ),
            ),
        ],
      ),
    ]);
  }

  /// Whether [person] is the host, which is all the "host" tag on a row
  /// is drawn from, on the host's machine and a guest's alike.
  bool _isHost(ShareState share, BridgeSharePerson person) =>
      // The engine says who the host is. This machine also knows it of
      // itself while it has not yet found the server, which is when nobody
      // has been told anything.
      person.host || (person.me && _hosting(share));

  /// The composition [person] has open, by name. The list of compositions
  /// is the one the shell keeps, so asking costs no call.
  String? _compOf(BridgeSharePerson person) {
    final open = person.comp?.internalid;
    if (open == null) return null;
    for (final (comp, name) in widget.app.comps()) {
      if (comp.internalid == open) return name;
    }
    return null;
  }
}

// --- The lists ------------------------------------------------------------

/// A small mono word after a name: you, host.
Widget _tag(LumitTheme t, String word) => Text(
      t.kickerCase(word),
      style: t.mono.copyWith(
          fontSize: dialogMonoSize, letterSpacing: 0.6, color: t.textDisabled),
    );

/// One person on the list: their colour and name, what they are to the
/// project, where they are, and at the end of the row the way to follow
/// them. For the host a way to take them out shows while the row is pointed
/// at or holds the keyboard, so the list reads as people and not as a column
/// of ways to be rid of them.
class _PersonRow extends StatefulWidget {
  final BridgeSharePerson person;

  /// They keep the project, they are only what was last heard, this machine
  /// is following them, they are being taken out, and something else is
  /// being asked of the server just now.
  /// What their disc carries, which is two letters when someone else has
  /// their initial.
  final String letters;

  /// They can look at the project and not change it.
  final bool look;
  final bool host, stale, followed, removing, busy;

  /// The composition they have open, by name.
  final String? comp;
  final VoidCallback? onOpenComp, onFollow, onRemove;

  const _PersonRow({
    super.key,
    required this.person,
    required this.letters,
    required this.host,
    required this.look,
    required this.stale,
    required this.comp,
    required this.onOpenComp,
    required this.followed,
    required this.onFollow,
    required this.removing,
    required this.busy,
    required this.onRemove,
  });

  @override
  State<_PersonRow> createState() => _PersonRowState();
}

class _PersonRowState extends State<_PersonRow> {
  bool _hover = false, _focus = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final person = widget.person;
    final colour = t.personColour(person.colour);
    final stale = widget.stale;
    final pointed = _hover || _focus || widget.removing;
    final follow = stale ? null : widget.onFollow;
    final remove = widget.onRemove;
    final comp = stale ? null : widget.comp;
    final place = comp == null
        ? null
        : Text(comp,
            style: t.small.copyWith(color: t.textSecondary),
            overflow: TextOverflow.ellipsis);
    return Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onFocusChange: (has) => setState(() => _focus = has),
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: SizedBox(
          height: _listRow,
          child: Row(
            children: [
              PersonDisc(
                  colour: person.colour,
                  name: person.name,
                  letters: widget.letters,
                  stale: stale),
              const SizedBox(width: 8),
              // Who, with what they are set apart from it.
              Expanded(
                child: Row(
                  children: [
                    Flexible(
                      child: Text(person.name,
                          style: stale
                              ? t.body.copyWith(color: t.textDisabled)
                              : t.body,
                          overflow: TextOverflow.ellipsis),
                    ),
                    if (person.me) ...[
                      const SizedBox(width: 6),
                      _tag(t, l10n.shareYou),
                    ],
                    if (widget.host) ...[
                      const SizedBox(width: 8),
                      _tag(t, l10n.shareHostTag),
                    ],
                    if (widget.look) ...[
                      const SizedBox(width: 8),
                      _tag(t, l10n.shareLookTag),
                    ],
                    // Straight after the name it is about. Anywhere nearer
                    // the composition it reads as removing that. There for
                    // the keyboard always, and for the eye only when the
                    // row is pointed at.
                    if (remove != null) ...[
                      const SizedBox(width: 12),
                      IgnorePointer(
                        ignoring: !pointed,
                        child: Opacity(
                          opacity: pointed ? 1 : 0,
                          child: HouseButton(
                            key: ValueKey<String>('share-remove-${person.id}'),
                            frameless: true,
                            small: true,
                            padding: EdgeInsets.zero,
                            onPressed: widget.busy ? null : remove,
                            child: Text(
                                widget.removing
                                    ? l10n.shareRemoving
                                    : l10n.shareRemove,
                                style: t.small.copyWith(color: t.textMuted)),
                          ),
                        ),
                      ),
                    ],
                  ],
                ),
              ),
              // Where, which opens it.
              if (place != null) ...[
                const SizedBox(width: 8),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 120),
                  child: switch (widget.onOpenComp) {
                    null => place,
                    final open => MouseRegion(
                        cursor: SystemMouseCursors.click,
                        child: GestureDetector(
                          key: ValueKey<String>('share-where-${person.id}'),
                          behavior: HitTestBehavior.opaque,
                          onTap: open,
                          child: place,
                        ),
                      ),
                  },
                ),
              ],
              // Following: quiet until the row is pointed at, and theirs
              // while it is on.
              SizedBox(
                width: 22,
                child: follow == null
                    ? null
                    : LumitTooltip(
                        message: widget.followed
                            ? l10n.shareFollowStop
                            : l10n.shareFollow(person.name),
                        child: MouseRegion(
                          cursor: SystemMouseCursors.click,
                          child: GestureDetector(
                            key: ValueKey<String>('share-follow-${person.id}'),
                            behavior: HitTestBehavior.opaque,
                            onTap: follow,
                            child: SizedBox(
                              height: _listRow,
                              child: Align(
                                alignment: Alignment.centerRight,
                                child: lumitIcon(
                                  LumitIcon.eye,
                                  size: 14,
                                  color: widget.followed || _hover
                                      ? colour
                                      : t.textSecondary.withValues(alpha: 0.4),
                                ),
                              ),
                            ),
                          ),
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

/// A list that grows to [maxHeight] and then scrolls, and says that it
/// does: an edge with more past it fades to the window's ground, and a thin
/// thumb shows where it has got to. With [free] it takes all the room it
/// needs, for when the window round it is the thing that scrolls. [gutter]
/// is room kept for the thumb beside rows that end in something to press.
class _ScrollList extends StatefulWidget {
  final double maxHeight;
  final bool free;
  final double gutter;

  /// Told when there is more than fits.
  final VoidCallback? onScrolls;
  final List<Widget> children;

  const _ScrollList({
    required this.maxHeight,
    this.free = false,
    this.gutter = 0,
    this.onScrolls,
    required this.children,
  });

  @override
  State<_ScrollList> createState() => _ScrollListState();
}

class _ScrollListState extends State<_ScrollList> {
  final ScrollController _scroll = ScrollController();
  bool _above = false, _below = false;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_read);
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  /// Which edges have more past them, as the list is laid out now.
  void _read() {
    if (!mounted || !_scroll.hasClients) return;
    final at = _scroll.position;
    final above = at.pixels > 0.5;
    final below = at.pixels < at.maxScrollExtent - 0.5;
    if (above || below) widget.onScrolls?.call();
    if (above != _above || below != _below) {
      setState(() {
        _above = above;
        _below = below;
      });
    }
  }

  Widget _fade(LumitTheme t, {required bool top}) => Positioned(
        left: 0,
        right: 0,
        top: top ? 0 : null,
        bottom: top ? null : 0,
        height: 16,
        child: IgnorePointer(
          child: DecoratedBox(
            decoration: BoxDecoration(
              gradient: LinearGradient(
                begin: top ? Alignment.topCenter : Alignment.bottomCenter,
                end: top ? Alignment.bottomCenter : Alignment.topCenter,
                colors: [t.surface1, t.surface1.withValues(alpha: 0)],
              ),
            ),
          ),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final column = Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: widget.children,
    );
    if (widget.free) return column;
    // Whether it scrolls is only known once it is laid out.
    WidgetsBinding.instance.addPostFrameCallback((_) => _read());
    return ConstrainedBox(
      constraints: BoxConstraints(maxHeight: widget.maxHeight),
      child: Stack(
        children: [
          RawScrollbar(
            controller: _scroll,
            interactive: false,
            thumbVisibility: true,
            thickness: 3,
            radius: const Radius.circular(2),
            thumbColor: t.surface4,
            child: ScrollConfiguration(
              behavior:
                  ScrollConfiguration.of(context).copyWith(scrollbars: false),
              child: SingleChildScrollView(
                controller: _scroll,
                padding: EdgeInsets.only(
                    right: _above || _below ? widget.gutter : 0),
                child: column,
              ),
            ),
          ),
          if (_above) _fade(t, top: true),
          if (_below) _fade(t, top: false),
        ],
      ),
    );
  }
}

// --- Exporting for someone ------------------------------------------------

/// Ask whether this computer should do the export someone else asked of it.
/// Yes puts it in the queue here and sends them the file when it is done.
Future<void> showExportAskFrb(BuildContext context, LumitState app,
    ({String job, int from, String comp}) ask) async {
  final who = app.share.nameOf(ask.from) ?? l10n.shareDefaultName;
  final yes = await showLumitModal<bool>(
    context: context,
    id: 'share-export-ask',
    builder: (close) => _Question(
      title: l10n.shareExportAskTitle(who),
      body: l10n.shareExportAskBody(who, ask.comp),
      no: l10n.shareExportAskNo,
      yes: l10n.shareExportAskYes,
      keyPrefix: 'share-export-ask',
      onChoose: close,
    ),
  );
  app.share.answerExport(ask.job, yes: yes == true);
}

/// A small question with two answers, the careful one the default.
class _Question extends StatelessWidget {
  final String title;
  final String body;
  final String no;
  final String yes;
  final String keyPrefix;
  final bool yesLive;
  final ValueChanged<bool?> onChoose;

  const _Question({
    required this.title,
    required this.body,
    required this.no,
    required this.yes,
    required this.keyPrefix,
    this.yesLive = true,
    required this.onChoose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 380,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(title, style: t.bodyPrimary),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(body, style: t.small.copyWith(color: t.textMuted)),
          ),
          const SizedBox(height: 14),
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: ValueKey<String>('$keyPrefix-no'),
                  small: true,
                  primary: true,
                  autofocus: true,
                  onPressed: () => onChoose(false),
                  child: Text(no),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: ValueKey<String>('$keyPrefix-yes'),
                  small: true,
                  onPressed: yesLive ? () => onChoose(true) : null,
                  child: Text(yes, style: t.small),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// --- Showing a link -------------------------------------------------------

/// How much room one speck of ink has to itself, in square logical pixels,
/// and how far it wanders from where it sits.
const double _inkRoom = 7;
const double _inkDrift = 1.6;

/// How long the ink takes to come back round to where it started.
const Duration _inkLoop = Duration(seconds: 7);

/// Something shown only when the person asks: a link, a password. While it
/// is hidden it is not drawn at all, so there is nothing of it in a
/// screenshot to work back from. Ink is drawn where it would be: specks
/// that drift, laid out by the size of the box and never by what is in it.
/// Showing it lets the ink lift away as the thing itself comes up.
///
/// [builder] is told how hidden the thing is, from 0 for shown to 1, and
/// has to draw nothing of it at 1. [lineHeight] lays the ink in lines that
/// tall, for something that wraps. [inset] keeps the ink off a field's edge.
class _Veil extends StatelessWidget {
  final bool shown;
  final double? lineHeight;
  final EdgeInsets inset;
  final Widget Function(double hidden) builder;

  const _Veil({
    required this.shown,
    this.lineHeight,
    this.inset = EdgeInsets.zero,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    final theme = ThemeScope.of(context);
    // Twice as long as one surface takes to give way to another: the ink
    // needs the time to be seen going. Still when the theme's motion is.
    // Hiding is instant. A fade in showed the first letters typed.
    final spec = theme.motion.swap;
    return TweenAnimationBuilder<double>(
      tween: Tween<double>(end: shown ? 0 : 1),
      duration: shown ? spec.duration * 2 : Duration.zero,
      curve: spec.curve,
      builder: (context, hidden, _) => Stack(
        children: [
          builder(hidden),
          if (hidden > 0)
            Positioned.fill(
              child: Padding(
                padding: inset,
                child: _Ink(
                  amount: hidden,
                  lineHeight: lineHeight,
                  colour: theme.theme.textSecondary,
                  // The drift is a loop, so only Full plays it.
                  still: !theme.motion.carries,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// The ink over something hidden. It takes no pointer, so a field under it
/// still takes a click and a paste.
class _Ink extends StatefulWidget {
  final double amount;
  final double? lineHeight;
  final Color colour;
  final bool still;

  const _Ink({
    required this.amount,
    required this.lineHeight,
    required this.colour,
    required this.still,
  });

  @override
  State<_Ink> createState() => _InkState();
}

class _InkState extends State<_Ink> with SingleTickerProviderStateMixin {
  late final AnimationController _loop =
      AnimationController(vsync: this, duration: _inkLoop);

  @override
  void initState() {
    super.initState();
    if (!widget.still) _loop.repeat();
  }

  @override
  void dispose() {
    _loop.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => IgnorePointer(
        child: RepaintBoundary(
          child: CustomPaint(
            painter: _InkPainter(
              loop: _loop,
              amount: widget.amount,
              lineHeight: widget.lineHeight,
              colour: widget.colour,
            ),
          ),
        ),
      );
}

class _InkPainter extends CustomPainter {
  final Animation<double> loop;
  final double amount;
  final double? lineHeight;
  final Color colour;

  _InkPainter({
    required this.loop,
    required this.amount,
    required this.lineHeight,
    required this.colour,
  }) : super(repaint: loop);

  /// A number from 0 to 1 that is always the same for the same speck and
  /// [salt], with no pattern from one speck to the next.
  static double _unit(int speck, int salt) {
    var h = (speck * 374761393 + salt * 668265263) & 0x7fffffff;
    h = ((h ^ (h >> 13)) * 1274126177) & 0x7fffffff;
    return ((h ^ (h >> 16)) & 0xffff) / 0x10000;
  }

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    final lines = lineHeight == null
        ? 1
        : math.max(1, (size.height / lineHeight!).round());
    final line = size.height / lines;
    // The ink sits where the letters would, not edge to edge of the line.
    final band = math.min(line * 0.6, 11.0);
    final each = (size.width * band / _inkRoom).round().clamp(8, 1500);
    final phase = loop.value;
    // Three weights of speck, each brightening and dimming in its own time.
    final specks = List.generate(3, (_) => <double>[]);
    for (var row = 0; row < lines; row++) {
      final middle = line * (row + 0.5);
      for (var i = 0; i < each; i++) {
        final speck = row * 7919 + i;
        final turn = 2 * math.pi * (phase * (1 + speck % 2) + _unit(speck, 3));
        // Going, each speck lifts by its own amount and spreads.
        final going = (1 - amount) * (2 + 6 * _unit(speck, 4));
        specks[speck % 3]
          ..add(_unit(speck, 1) * size.width +
              math.cos(turn) * (_inkDrift + going * 0.5))
          ..add(middle +
              (_unit(speck, 2) - 0.5) * band +
              math.sin(turn) * _inkDrift * 0.6 -
              going);
      }
    }
    for (final (weight, points) in specks.indexed) {
      final glow = 0.5 + 0.5 * math.sin(2 * math.pi * (phase + weight / 3));
      final paint = Paint()
        ..color = colour.withValues(alpha: amount * (0.3 + 0.35 * glow))
        ..strokeWidth = 1.1 + 0.3 * weight
        ..strokeCap = StrokeCap.round;
      canvas.drawRawPoints(
          PointMode.points, Float32List.fromList(points), paint);
    }
  }

  @override
  bool shouldRepaint(_InkPainter old) =>
      old.amount != amount ||
      old.lineHeight != lineHeight ||
      old.colour != colour;
}

/// Asked before a hidden link is shown: what it gives away, and to whom.
class _ShowLinkQuestion extends StatelessWidget {
  /// The link is to a project kept online, which names no machine, and
  /// with [look] it is the invite that can only look.
  final bool online, look;
  final ValueChanged<bool?> onChoose;

  const _ShowLinkQuestion(
      {required this.online, this.look = false, required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 380,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(l10n.shareShowTitle, style: t.bodyPrimary),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(
                look
                    ? l10n.shareShowBodyLook
                    : online
                        ? l10n.shareShowBodyOnline
                        : l10n.shareShowBody,
                style: t.small.copyWith(color: t.textMuted)),
          ),
          const SizedBox(height: 14),
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('share-show-cancel'),
                  small: true,
                  primary: true,
                  autofocus: true,
                  onPressed: () => onChoose(false),
                  child: Text(l10n.shareKeepHidden),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('share-show-confirm'),
                  small: true,
                  onPressed: () => onChoose(true),
                  child: Text(l10n.shareShowConfirm, style: t.small),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// --- Conflicts ------------------------------------------------------------

/// Open the Conflicts window: this guest's changes that touched something
/// another person changed while the host was out of reach.
Future<void> showShareConflictsFrb(BuildContext context, LumitState app,
        {Future<String?> Function()? savePicker}) =>
    showLumitModal<void>(
      context: context,
      id: 'share-conflicts',
      builder: (close) => _ConflictsDialog(
          app: app, savePicker: savePicker, onClose: () => close(null)),
    );

class _ConflictsDialog extends StatefulWidget {
  final LumitState app;

  /// Stands in for the system's save window, as it does for saving.
  final Future<String?> Function()? savePicker;
  final VoidCallback onClose;

  const _ConflictsDialog(
      {required this.app, this.savePicker, required this.onClose});

  @override
  State<_ConflictsDialog> createState() => _ConflictsDialogState();
}

class _ConflictsDialogState extends State<_ConflictsDialog> {
  List<BridgeShareConflict> _rows = const [];

  @override
  void initState() {
    super.initState();
    _rows = widget.app.share.conflicts();
  }

  /// Settle one, then read the list again: the rest have moved up a place.
  void _choose(int index, {required bool mine}) {
    final share = widget.app.share;
    final refused = share.resolve(index, mine: mine);
    if (refused > 0) {
      widget.app.postNotice(l10n.shareResolveRefused(refused), error: true);
    }
    final left = share.conflicts();
    if (left.isEmpty) return widget.onClose();
    setState(() => _rows = left);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: shareDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.shareConflictsTitle,
          onClose: widget.onClose,
          keyPrefix: 'conflicts',
        ),
        _body([
          Text(l10n.shareConflictsIntro, style: t.body),
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: _conflictListHeight),
            child: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (index, conflict) in _rows.indexed)
                    _row(t, index, conflict),
                ],
              ),
            ),
          ),
        ]),
        dialogFooter(
          t,
          keyPrefix: 'conflicts',
          // This person's own project from before the merge that left
          // these, should neither answer do.
          leading: switch (widget.app.share.returnCopies.firstOrNull) {
            null => null,
            final latest => HouseButton(
                key: const ValueKey('conflicts-open-copy'),
                frameless: true,
                padding: const EdgeInsets.symmetric(horizontal: 4),
                onPressed: () => unawaited(openReturnCopy(
                    context, widget.app, latest, widget.onClose,
                    picker: widget.savePicker)),
                child: Text(l10n.shareOpenMyCopy,
                    style: t.small.copyWith(color: t.textMuted)),
              ),
          },
          actions: [
            HouseButton(
              key: const ValueKey('conflicts-close'),
              small: true,
              primary: true,
              onPressed: widget.onClose,
              child: Text(l10n.close),
            ),
          ],
        ),
      ],
    );
  }

  /// One conflict: the step, where it was and how much, and the two answers.
  Widget _row(LumitTheme t, int index, BridgeShareConflict conflict) =>
      Padding(
        padding: const EdgeInsets.only(top: 10),
        child: Row(
          children: [
            Expanded(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(engineLabel(conflict.step),
                      style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
                  Text(
                    [
                      conflict.item,
                      conflict.layer,
                      l10n.shareConflictEdits(conflict.edits),
                    ].nonNulls.join(' · '),
                    style: t.small.copyWith(color: t.textMuted),
                    overflow: TextOverflow.ellipsis,
                  ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            HouseButton(
              key: ValueKey<String>('conflicts-mine-$index'),
              small: true,
              onPressed: () => _choose(index, mine: true),
              child: Text(l10n.shareKeepMine, style: t.small),
            ),
            const SizedBox(width: 6),
            HouseButton(
              key: ValueKey<String>('conflicts-theirs-$index'),
              small: true,
              onPressed: () => _choose(index, mine: false),
              child: Text(l10n.shareKeepTheirs, style: t.small),
            ),
          ],
        ),
      );
}
