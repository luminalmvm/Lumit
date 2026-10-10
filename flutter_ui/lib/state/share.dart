// A shared project as the interface sees it: which end this is, who else is
// here and what they are looking at, and whether the host can be reached.
// LumitState owns one and drives it. Panels listen and draw.

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';

/// Why a guest was turned away or sharing ended, as a sentence to show.
String shareEndingText(BridgeShareEnding reason) => switch (reason) {
      BridgeShareEnding_Closed() => l10n.shareEndedClosed,
      BridgeShareEnding_VersionMismatch(:final host) =>
        l10n.shareVersionMismatch(host),
      BridgeShareEnding_Full() => l10n.shareFull,
      BridgeShareEnding_Unsafe() => l10n.shareUnsafe,
      BridgeShareEnding_Removed() => l10n.shareRemoved,
    };

/// Which end of a shared project this is.
enum ShareRole { none, host, guest }

class ShareState extends ChangeNotifier {
  ShareRole role = ShareRole.none;

  /// The port this machine listens on while it hosts.
  int? port;

  /// Everyone in the project, this person included. This notifier fires on
  /// every change to it, a moving playhead or pointer included, so listen to
  /// it only to draw those. Everything else listens to [roster].
  List<BridgeSharePerson> people = const [];

  /// Moves on when who is here, which composition they have open or what
  /// they have selected changes, and when the host is lost or found or a
  /// conflict is settled. Not for a playhead or a pointer. What a panel that
  /// marks rows listens to, so it is not rebuilt twenty times a second while
  /// someone else scrubs.
  final ValueNotifier<int> roster = ValueNotifier(0);

  String _rosterWas = '';

  void _noteRoster() {
    final now = people
        .map((p) => '${p.id}|${p.name}|${p.colour}|${p.me}'
            '|${p.comp?.internalid}|${p.layers.map((l) => l.internallayerId)}')
        .join(';');
    if (now == _rosterWas) return;
    _rosterWas = now;
    roster.value++;
  }

  /// This guest has lost its host and is working on alone until it is back.
  bool away = false;

  /// How many conflicts a merge has left waiting to be chosen between.
  int held = 0;

  ProjectReference? _project;

  bool get active => role != ShareRole.none;

  /// Everyone but this person, which is who the panels draw.
  Iterable<BridgeSharePerson> get others => people.where((p) => !p.me);

  /// The others who have [comp] open.
  Iterable<BridgeSharePerson> inComp(CompositionReference? comp) =>
      comp == null
          ? const []
          : others.where((p) => p.comp?.internalid == comp.internalid);

  void begin(ShareRole as, ProjectReference project, {int? onPort}) {
    role = as;
    port = onPort;
    _project = project;
    people = project.sharePeople();
    away = false;
    held = 0;
    _sent = null;
    _noteRoster();
    // A guest's copy opened again brings the conflicts it was closed with.
    held = project.shareConflicts().length;
    notifyListeners();
    _send();
  }

  /// Sharing is over, or the project it was for has gone.
  void end() {
    if (!active) return;
    role = ShareRole.none;
    port = null;
    _project = null;
    people = const [];
    away = false;
    held = 0;
    _noteRoster();
    notifyListeners();
  }

  void setPeople(List<BridgeSharePerson> now) {
    people = now;
    _noteRoster();
    notifyListeners();
  }

  void setAway(bool now, {int? conflicts}) {
    away = now;
    if (conflicts != null) held = conflicts;
    roster.value++;
    notifyListeners();
  }

  /// Take the guest the list knows as [person] out of the project. For the
  /// host, and nothing happens for anyone else.
  void remove(int person) {
    try {
      _project?.shareRemove(person: person);
    } catch (_) {
      // The project closed between the click and the call.
    }
  }

  /// Look for a lost host by a new invite. False when [invite] is not one.
  bool reinvite(String invite) {
    try {
      return _project?.shareReinvite(invite: invite) ?? false;
    } catch (_) {
      return false;
    }
  }

  /// The conflicts waiting, to list in the dialogue that settles them.
  List<BridgeShareConflict> conflicts() =>
      _project?.shareConflicts() ?? const [];

  /// Settle the conflict at [index], keeping this person's edits or the
  /// host's. Answers how many of this person's edits no longer applied.
  int resolve(int index, {required bool mine}) {
    final project = _project;
    if (project == null) return 0;
    final failed = project.shareResolve(index: index, mine: mine);
    held = project.shareConflicts().length;
    roster.value++;
    notifyListeners();
    return failed;
  }

  // What this person is looking at. The engine sends it on latest-wins, so
  // these only have to keep it from being told the same thing twice.
  CompositionReference? _comp;
  List<LayerReference> _layers = const [];
  int? _playhead;
  double? _x, _y;
  String? _sent;

  /// The composition open, the layers selected in it and the playhead's
  /// frame. Does nothing unless the project is shared.
  void look(
      {required CompositionReference? comp,
      required List<LayerReference> layers,
      required int? playhead}) {
    _comp = comp;
    _layers = layers;
    _playhead = playhead;
    _send();
  }

  /// The pointer over the Viewer in composition pixels, or nulls once it has
  /// left. Does nothing unless the project is shared.
  void point(double? x, double? y) {
    _x = x;
    _y = y;
    _send();
  }

  void _send() {
    final project = _project;
    if (project == null) return;
    final layers = _comp == null ? const <LayerReference>[] : _layers;
    final now = '${_comp?.internalid}|${layers.map((l) => l.internallayerId)}'
        '|$_playhead|${_x?.round()}|${_y?.round()}';
    if (now == _sent) return;
    _sent = now;
    try {
      project.sharePresence(
          comp: _comp,
          layers: layers,
          playhead: _playhead,
          cursorX: _x,
          cursorY: _y);
    } catch (_) {
      // The project closed between the gesture and the call.
    }
  }
}
