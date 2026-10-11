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
import 'dart:math' as math;
import 'dart:typed_data' show Float32List;
import 'dart:ui' show PointMode;

import 'package:flutter/material.dart' show SelectableText;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/file_dialogs.dart';
import '../state/share.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'pro_window.dart';

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
  TextStyle? style,
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
        {String? invite, String? relay}) =>
    showLumitModal<void>(
      context: context,
      id: 'share',
      builder: (close) => _ShareDialog(
          app: app, invite: invite, relay: relay, onClose: () => close(null)),
    );

/// The two things there are to do before a project is shared.
enum _Page { share, join }

class _ShareDialog extends StatefulWidget {
  final LumitState app;
  final String? invite;
  final String? relay;
  final VoidCallback onClose;

  const _ShareDialog(
      {required this.app, this.invite, this.relay, required this.onClose});

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

  /// Why the last thing asked for did not happen, to show under the fields.
  String? _error;

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
    if (_prefs.shareHosted[_projectId] case final kept?) {
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
    _link = _linkNow();
  }

  @override
  void dispose() {
    widget.app.share.roster.removeListener(_readLink);
    _name.dispose();
    _port.dispose();
    _relay.dispose();
    _address.dispose();
    _joinInvite.dispose();
    _newInvite.dispose();
    _password.dispose();
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
    final link = _linkNow();
    if (link != _link && mounted) setState(() => _link = link);
  }

  Future<void> _start() async {
    // Anything that is not a port number asks for the usual one.
    final asked =
        (int.tryParse(_port.text.trim()) ?? _defaultPort).clamp(0, 65535);
    var relay = _relay.text.trim();
    if (relay != (_prefs.shareRelay ?? '')) {
      _prefs.setShareRelay(relay.isEmpty ? null : relay);
    }
    // With no relay of the person's own, an account with Pro uses Lumit's,
    // which is reached through a door that has to be open first.
    if (relay.isEmpty &&
        _prefs.shareCloud &&
        _ui.account.pro &&
        await _ui.relayDoor.open()) {
      relay = shareCloudRelay();
    }
    if (!mounted) return;
    final started = widget.app.startSharing(
        name: _nameNow(),
        port: asked,
        key: _keptKey,
        password: _passwordNow(),
        outside: _outside,
        relay: relay.isEmpty ? null : relay);
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
    });
    final password = _needsPassword ? _passwordNow() : null;
    // The host may be at Lumit's relay, which a guest needs no account for
    // but does need the door to.
    await _ui.relayDoor.open();
    final outcome = await widget.app.joinShared(
        invite: _joinInvite.text.trim(),
        name: _nameNow(),
        password: password,
        footage: _footage);
    if (outcome is BridgeJoinOutcome_Joined) {
      widget.onClose();
      return;
    }
    if (!mounted) return;
    setState(() {
      _joining = false;
      _error = switch (outcome) {
        BridgeJoinOutcome_BadInvite() => l10n.shareBadInvite,
        BridgeJoinOutcome_PasswordNeeded() => l10n.sharePasswordNeeded,
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

  /// Copying needs no look at the link, so it never asks.
  void _copy() {
    Clipboard.setData(ClipboardData(text: _link ?? ''));
    widget.app.postNotice(l10n.shareLinkCopied);
  }

  /// Show the links in this window, once the person has been told what
  /// showing one risks, or hide them again.
  Future<void> _toggleShown() async {
    if (_shown) return setState(() => _shown = false);
    final agreed = await showLumitModal<bool>(
      context: context,
      id: 'share-show',
      builder: (close) => _ShowLinkQuestion(onChoose: close),
    );
    if (agreed == true && mounted) setState(() => _shown = true);
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
      listenable: Listenable.merge([share.roster, share.footage]),
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

  /// The fold the rows most people never need sit under.
  Widget _advancedFold(LumitTheme t) => Padding(
        padding: const EdgeInsets.only(top: 8),
        child: GestureDetector(
          key: const ValueKey('share-advanced'),
          behavior: HitTestBehavior.opaque,
          onTap: () => setState(() => _advanced = !_advanced),
          child: Row(
            children: [
              TwirlTurn(
                open: _advanced,
                child: lumitIcon(
                  _advanced ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                  size: 12,
                  color: t.textMuted,
                ),
              ),
              const SizedBox(width: 4),
              Text(l10n.shareAdvanced,
                  style: t.small.copyWith(color: t.textSecondary)),
            ],
          ),
        ),
      );

  /// A link, hidden until the person asks to see it. While it is hidden it
  /// is not drawn, read out or selectable: the well holds only ink, and the
  /// room the link takes up.
  Widget _linkWell(LumitTheme t, String id, String link) => Container(
        padding: const EdgeInsets.all(8),
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.wellRadius),
        ),
        child: _Veil(
          shown: _shown,
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
        builder: (hidden) => _field(t, id, controller,
            hint: hint,
            onSubmitted: onSubmitted,
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

  Widget _showButton(LumitTheme t) => HouseButton(
        key: const ValueKey('share-show'),
        small: true,
        onPressed: _toggleShown,
        child: Text(_shown ? l10n.shareHideLink : l10n.shareShowLink,
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
  List<Widget> _footageRows(LumitTheme t) => [
        const SizedBox(height: 6),
        dialogRow(
          t,
          l10n.shareFootage,
          Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              _tick(t, 'share-give', l10n.shareGive, _prefs.shareGive, (on) {
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
        _limitRow(t, l10n.shareUpLimit, 'share-up-limit', _upLimit),
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
        _line(t, l10n.sharePasswordHint),
        _cloudRow(t),
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

  /// Lumit's own relay: a tick for an account with Pro, and for anyone else
  /// what Pro would do here and the way to it.
  Widget _cloudRow(LumitTheme t) => ListenableBuilder(
        listenable: _ui.account,
        builder: (context, _) => dialogRow(
          t,
          l10n.shareCloud,
          _ui.account.pro
              ? Row(
                  children: [
                    HouseCheckbox(
                      key: const ValueKey('share-cloud'),
                      value: _prefs.shareCloud,
                      onChanged: (on) =>
                          setState(() => _prefs.setShareCloud(on)),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                        child: Text(l10n.shareCloudUse, style: t.small)),
                  ],
                )
              : Row(
                  children: [
                    Expanded(
                      child: Text(l10n.shareCloudPro,
                          style: t.small.copyWith(color: t.textMuted)),
                    ),
                    const SizedBox(width: 8),
                    HouseButton(
                      key: const ValueKey('share-cloud-pro'),
                      small: true,
                      onPressed: () => unawaited(showProWindow(context)),
                      child: Text(l10n.shareCloudSee, style: t.small),
                    ),
                  ],
                ),
          labelColumn: _labelColumn,
        ),
      );

  /// Joining somebody else's: their link, and where the footage is here.
  List<Widget> _joinRows(LumitTheme t) => [
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
        _line(t, l10n.shareFootageHint),
        _advancedFold(t),
        if (_advanced) ..._footageRows(t),
        if (_error case final error?) _line(t, error, warning: true),
      ];

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
          if (_needsPassword)
            _passwordRow(t, l10n.sharePasswordTheirs, _reinvite),
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
  final ValueChanged<bool?> onChoose;

  const _Question({
    required this.title,
    required this.body,
    required this.no,
    required this.yes,
    required this.keyPrefix,
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
                  onPressed: () => onChoose(true),
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
    final spec = theme.motion.swap;
    return TweenAnimationBuilder<double>(
      tween: Tween<double>(end: shown ? 0 : 1),
      duration: spec.duration * 2,
      curve: Curves.easeInOut,
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
                  still: spec.isStill,
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
  final ValueChanged<bool?> onChoose;

  const _ShowLinkQuestion({required this.onChoose});

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
            child: Text(l10n.shareShowBody,
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
