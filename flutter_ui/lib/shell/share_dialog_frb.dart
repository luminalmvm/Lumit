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

import 'package:flutter/material.dart' show SelectableText;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';
import 'package:provider/provider.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/file_dialogs.dart';
import '../state/share.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// The width both take, the column their labels sit in, and how tall the list
/// of conflicts grows before it scrolls.
const double shareDialogWidth = 400;
const double _labelColumn = 100;
const double _conflictListHeight = 320;

/// A person's colour as a dot.
Widget sharePersonDot(LumitTheme t, BridgeSharePerson person) => Container(
      width: 8,
      height: 8,
      decoration: BoxDecoration(
          color: t.personColour(person.colour), shape: BoxShape.circle),
    );

/// A text well at the dialog's control height, filling its row.
Widget _field(
  LumitTheme t,
  String id,
  TextEditingController controller, {
  String? hint,
  VoidCallback? onSubmitted,
}) =>
    SizedBox(
      height: dialogControlHeight,
      child: HouseTextField(
        key: ValueKey<String>(id),
        controller: controller,
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        fill: t.surface0,
        hint: hint,
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
/// else's, or see who is here.
Future<void> showShareFrb(BuildContext context, LumitState app) =>
    showLumitModal<void>(
      context: context,
      id: 'share',
      builder: (close) => _ShareDialog(app: app, onClose: () => close(null)),
    );

/// The two things there are to do before a project is shared.
enum _Page { share, join }

class _ShareDialog extends StatefulWidget {
  final LumitState app;
  final VoidCallback onClose;

  const _ShareDialog({required this.app, required this.onClose});

  @override
  State<_ShareDialog> createState() => _ShareDialogState();
}

class _ShareDialogState extends State<_ShareDialog> {
  late final Workspace _prefs;
  late final TextEditingController _name;
  late final TextEditingController _port;
  late final TextEditingController _address;
  late final int _defaultPort;

  /// What a guest pastes: the invite to join by, and a fresh one for a host
  /// that has moved.
  final TextEditingController _joinInvite = TextEditingController();
  final TextEditingController _newInvite = TextEditingController();

  _Page _page = _Page.share;

  /// What the engine knows the open project by, which the secret of its
  /// invite is filed under, and that secret if it was shared from here before.
  String? _projectId;
  String? _keptKey;

  /// This time's invite is the one handed out last time.
  bool _sameInvite = false;

  /// This machine's address on its own network, which the address field
  /// starts as and people on that network join by.
  late final String _localAddress;

  /// Whether sharing asks the router to open the port.
  late bool _outside;

  /// The invite for the address in the field, while this machine hosts.
  String? _invite;

  /// Where this machine keeps the footage of a project it is joining.
  String? _footage;

  /// The host is being asked and the project fetched.
  bool _joining = false;

  /// Why the last thing asked for did not happen, to show under the fields.
  String? _error;

  @override
  void initState() {
    super.initState();
    _prefs = context.read<LumitUiState>().workspace;
    _defaultPort = shareDefaultPort();
    int? keptPort;
    try {
      _projectId = widget.app.project?.shareId();
    } catch (_) {
      // No project to share. The Join page needs none.
    }
    if (_prefs.shareHosted[_projectId] case final kept?) {
      final cut = kept.indexOf('/');
      keptPort = int.tryParse(kept.substring(0, cut < 0 ? 0 : cut));
      _keptKey = kept.substring(cut + 1);
    }
    if (widget.app.project == null) _page = _Page.join;
    _name = TextEditingController(text: _prefs.shareName);
    _port = TextEditingController(text: '${keptPort ?? _defaultPort}');
    _localAddress = shareLocalAddress();
    _outside = _prefs.shareOutside;
    _address = TextEditingController(text: _localAddress)
      ..addListener(_readInvite);
    widget.app.share.roster.addListener(_readReach);
    _readReach();
    _invite = _inviteNow();
  }

  /// Once the router has opened the port, the invite carries the address it
  /// has on the internet, unless another has been typed over this machine's.
  void _readReach() {
    if (widget.app.share.reach case BridgeShareReach_Open(:final address)) {
      if (_address.text == _localAddress) _address.text = address;
    }
  }

  @override
  void dispose() {
    widget.app.share.roster.removeListener(_readReach);
    _name.dispose();
    _port.dispose();
    _address.dispose();
    _joinInvite.dispose();
    _newInvite.dispose();
    super.dispose();
  }

  /// The name the others see: what was typed, kept for next time, or the
  /// default when the field was left empty.
  String _nameNow() {
    final name = _name.text.trim();
    if (name != (_prefs.shareName ?? '')) {
      _prefs.setShareName(name.isEmpty ? null : name);
    }
    return name.isEmpty ? l10n.shareDefaultName : name;
  }

  String? _inviteNow() {
    try {
      return widget.app.project?.shareInvite(address: _address.text);
    } catch (_) {
      return null;
    }
  }

  void _readInvite() {
    final invite = _inviteNow();
    if (invite != _invite) setState(() => _invite = invite);
  }

  void _start() {
    // Anything that is not a port number asks for the usual one.
    final asked =
        (int.tryParse(_port.text.trim()) ?? _defaultPort).clamp(0, 65535);
    final started = widget.app.startSharing(
        name: _nameNow(), port: asked, key: _keptKey, outside: _outside);
    if (started case BridgeShareStarted_Sharing(:final port, :final key)) {
      // Kept until sharing is stopped, so the same project shared again after
      // a restart is found by the invite people already hold.
      if (_projectId case final id?) _prefs.setShareHosted(id, '$port/$key');
      _sameInvite = key == _keptKey;
      _keptKey = key;
    }
    setState(() {
      _error = switch (started) {
        BridgeShareStarted_Sharing() => null,
        BridgeShareStarted_PortInUse() => l10n.sharePortInUse(asked),
        _ => l10n.shareCouldNotStart,
      };
      _invite = _inviteNow();
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
    });
    final outcome = await widget.app.joinShared(
        invite: _joinInvite.text.trim(), name: _nameNow(), footage: _footage);
    if (outcome is BridgeJoinOutcome_Joined) {
      widget.onClose();
      return;
    }
    if (!mounted) return;
    setState(() {
      _joining = false;
      _error = switch (outcome) {
        BridgeJoinOutcome_BadInvite() => l10n.shareBadInvite,
        BridgeJoinOutcome_Unreachable() => l10n.shareUnreachable,
        BridgeJoinOutcome_VersionMismatch(:final host) =>
          l10n.shareVersionMismatch(host),
        BridgeJoinOutcome_Full() => l10n.shareFull,
        BridgeJoinOutcome_Unsafe() => l10n.shareUnsafe,
        _ => l10n.shareCouldNotJoin,
      };
    });
  }

  void _copy() {
    Clipboard.setData(ClipboardData(text: _invite ?? ''));
    widget.app.postNotice(l10n.shareInviteCopied);
  }

  /// Look for a lost host by the invite just pasted.
  void _reinvite() {
    final taken = widget.app.share.reinvite(_newInvite.text.trim());
    if (taken) _newInvite.clear();
    setState(() => _error = taken ? null : l10n.shareBadInvite);
  }

  /// Take a guest out. The engine replaces the invite as it does, so the one
  /// they hold stops working. The new one is shown, and its key is kept for
  /// sharing this project again.
  void _remove(int person) {
    final share = widget.app.share;
    share.remove(person);
    final invite = _inviteNow();
    final key = invite?.substring(invite.lastIndexOf('/') + 1);
    final (id, port) = (_projectId, share.port);
    if (key != null && id != null && port != null) {
      _prefs.setShareHosted(id, '$port/$key');
    }
    setState(() {
      _invite = invite;
      _keptKey = key ?? _keptKey;
      _sameInvite = false;
    });
  }

  /// Stop sharing for everyone, or leave. A host's invite is finished with,
  /// so the next time this project is shared it gets a new one.
  void _stop() {
    final id = _projectId;
    if (widget.app.share.role == ShareRole.host && id != null) {
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
      listenable: share.roster,
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
          _body(switch ((share.role, _page)) {
            (ShareRole.none, _Page.share) => _startRows(t),
            (ShareRole.none, _Page.join) => _joinRows(t),
            (ShareRole.host, _) => [..._inviteRows(t), _people(t, share)],
            (ShareRole.guest, _) => [..._guestRows(t, share), _people(t, share)],
          }),
          dialogFooter(
            t,
            keyPrefix: 'share',
            actions: [
              switch ((share.role, _page)) {
                (ShareRole.none, _Page.share) => HouseButton(
                    key: const ValueKey('share-start'),
                    primary: true,
                    autofocus: true,
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    onPressed: widget.app.project == null ? null : _start,
                    child: Text(l10n.shareStart),
                  ),
                (ShareRole.none, _Page.join) => HouseButton(
                    key: const ValueKey('share-join'),
                    primary: true,
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    onPressed: _joining ? null : _join,
                    child: Text(_joining ? l10n.shareJoining : l10n.shareJoin),
                  ),
                (ShareRole.host, _) => HouseButton(
                    key: const ValueKey('share-stop'),
                    padding: const EdgeInsets.symmetric(horizontal: 12),
                    onPressed: _stop,
                    child: Text(l10n.shareStop),
                  ),
                (ShareRole.guest, _) => HouseButton(
                    key: const ValueKey('share-leave'),
                    padding: const EdgeInsets.symmetric(horizontal: 12),
                    onPressed: _stop,
                    child: Text(l10n.shareLeave),
                  ),
              },
            ],
          ),
        ],
      ),
    );
  }

  Widget _nameRow(LumitTheme t, VoidCallback onSubmitted) => dialogRow(
        t,
        l10n.shareYourName,
        _field(t, 'share-name', _name,
            hint: l10n.shareDefaultName, onSubmitted: onSubmitted),
        labelColumn: _labelColumn,
      );

  /// Sharing the open project: who this person is and where to listen.
  List<Widget> _startRows(LumitTheme t) => [
        _nameRow(t, _start),
        dialogRow(
          t,
          l10n.sharePort,
          _field(t, 'share-port', _port, onSubmitted: _start),
          labelColumn: _labelColumn,
        ),
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
        _line(t, l10n.shareHostHint),
        if (_error case final error?) _line(t, error, warning: true),
      ];

  /// Joining somebody else's: their invite, and where the footage is here.
  List<Widget> _joinRows(LumitTheme t) => [
        _nameRow(t, _join),
        dialogRow(
          t,
          l10n.shareInvite,
          _field(t, 'share-join-invite', _joinInvite, onSubmitted: _join),
          labelColumn: _labelColumn,
        ),
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
        _line(t, l10n.shareFootageHint),
        if (_error case final error?) _line(t, error, warning: true),
      ];

  /// While hosting: the address others reach this machine at, and the invite
  /// that carries it.
  List<Widget> _inviteRows(LumitTheme t) => [
        dialogRow(
          t,
          l10n.shareAddress,
          _field(t, 'share-address', _address),
          labelColumn: _labelColumn,
        ),
        dialogRow(
          t,
          l10n.shareInvite,
          Align(
            alignment: Alignment.centerRight,
            child: HouseButton(
              key: const ValueKey('share-copy'),
              small: true,
              onPressed: _copy,
              child: Text(l10n.shareCopyInvite, style: t.small),
            ),
          ),
          labelColumn: _labelColumn,
        ),
        // On a line of its own: an invite is far longer than a row's control.
        Container(
          padding: const EdgeInsets.all(8),
          decoration: BoxDecoration(
            color: t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.wellRadius),
          ),
          child: SelectableText(
            _invite ?? '',
            key: const ValueKey('share-invite'),
            style: t.mono,
            selectionColor: t.accent.withValues(alpha: 0.5),
          ),
        ),
        if (_sameInvite) _line(t, l10n.shareSameInvite),
        switch (widget.app.share.reach) {
          BridgeShareReach_Off() => _line(t, l10n.shareReachOff),
          BridgeShareReach_Asking() => _line(t, l10n.shareReachAsking),
          BridgeShareReach_Open() =>
            _line(t, l10n.shareReachOpen(_localAddress)),
          BridgeShareReach_Refused() =>
            _line(t, l10n.shareReachRefused, warning: true),
          BridgeShareReach_Behind() =>
            _line(t, l10n.shareReachBehind, warning: true),
        },
        const SizedBox(height: dialogGroupGap),
      ];

  /// While a guest: what wants doing about a host out of reach, and about
  /// conflicts a merge left.
  List<Widget> _guestRows(LumitTheme t, ShareState share) => [
        if (share.away) ...[
          Text(l10n.shareAway, style: t.small.copyWith(color: t.warning)),
          const SizedBox(height: 6),
          dialogRow(
            t,
            l10n.shareNewInvite,
            Row(
              children: [
                Expanded(
                  child: _field(t, 'share-reinvite', _newInvite,
                      onSubmitted: _reinvite),
                ),
                const SizedBox(width: 8),
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
          if (_error case final error?) _line(t, error, warning: true),
        ],
        if (share.held > 0)
          dialogRow(
            t,
            l10n.shareConflictsTitle,
            Row(
              children: [
                Expanded(
                  child: Text(l10n.shareConflictsWaiting(share.held),
                      style: t.small),
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
          ),
        if (share.away || share.held > 0)
          const SizedBox(height: dialogGroupGap),
      ];

  /// Everyone in the project, this person marked, and for the host a way to
  /// take a guest out.
  Widget _people(LumitTheme t, ShareState share) =>
      dialogGroup(t, l10n.sharePeople, [
        for (final person in share.people)
          ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 24),
            child: Row(
              children: [
                sharePersonDot(t, person),
                const SizedBox(width: 8),
                Expanded(
                  child: Row(
                    children: [
                      Flexible(
                        child: Text(person.name,
                            style: t.body, overflow: TextOverflow.ellipsis),
                      ),
                      if (person.me) ...[
                        const SizedBox(width: 6),
                        Text(l10n.shareYou,
                            style: t.small.copyWith(color: t.textMuted)),
                      ],
                    ],
                  ),
                ),
                if (share.role == ShareRole.host && !person.me)
                  HouseButton(
                    key: ValueKey<String>('share-remove-${person.id}'),
                    small: true,
                    onPressed: () => _remove(person.id),
                    child: Text(l10n.shareRemove, style: t.small),
                  ),
              ],
            ),
          ),
      ]);
}

// --- Conflicts ------------------------------------------------------------

/// Open the Conflicts window: this guest's changes that touched something
/// another person changed while the host was out of reach.
Future<void> showShareConflictsFrb(BuildContext context, LumitState app) =>
    showLumitModal<void>(
      context: context,
      id: 'share-conflicts',
      builder: (close) =>
          _ConflictsDialog(app: app, onClose: () => close(null)),
    );

class _ConflictsDialog extends StatefulWidget {
  final LumitState app;
  final VoidCallback onClose;

  const _ConflictsDialog({required this.app, required this.onClose});

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
