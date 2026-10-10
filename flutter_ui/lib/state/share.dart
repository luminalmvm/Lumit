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
            '|${p.comp?.internalid}|${p.layers.map((l) => l.internallayerId)}'
            // A hash each, because a marquee can hold thousands of keyframes
            // and this is run for every move of anyone's playhead.
            '|${Object.hashAll(p.properties)}|${Object.hashAll(p.keys)}')
        .join(';');
    if (now == _rosterWas) return;
    _rosterWas = now;
    roster.value++;
  }

  /// This guest has lost its host and is working on alone until it is back.
  bool away = false;

  /// For a host: whether people outside its network can get in.
  BridgeShareReach reach = const BridgeShareReach.off();

  /// For a host: whether it has a room at a relay.
  BridgeShareRelayed relayed = BridgeShareRelayed.off;

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

  /// What the others have in hand in [comp], for a panel to mark its rows
  /// by: by layer id or row path, the colour of each person who has it. A
  /// layer is in it too when it is a row or a keyframe on it they hold, so a
  /// row that is out of sight still shows on its layer. Empty when the
  /// project is not shared.
  Map<String, List<int>> inHand(CompositionReference? comp) {
    if (!active) return const {};
    final out = <String, List<int>>{};
    for (final person in inComp(comp)) {
      void mark(String name) {
        final holders = out[name] ??= [];
        if (!holders.contains(person.colour)) holders.add(person.colour);
      }

      for (final layer in person.layers) {
        mark(layer.internallayerId.toString());
      }
      for (final path in [...person.properties, ...person.keys]) {
        final slash = path.indexOf('/');
        if (slash > 0) mark(path.substring(0, slash));
      }
      person.properties.forEach(mark);
    }
    return out;
  }

  /// The colour of each other person who has [name] in hand, whichever
  /// composition they have open: what the Project panel marks an item by.
  List<int> holding(String name) {
    if (!active) return const [];
    return [
      for (final person in others)
        if (person.properties.contains(name)) person.colour,
    ];
  }

  void begin(ShareRole as, ProjectReference project, {int? onPort}) {
    role = as;
    port = onPort;
    _project = project;
    people = project.sharePeople();
    away = false;
    held = 0;
    reach = as == ShareRole.host
        ? project.shareReach()
        : const BridgeShareReach.off();
    relayed = as == ShareRole.host
        ? project.shareRelayed()
        : BridgeShareRelayed.off;
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
    reach = const BridgeShareReach.off();
    relayed = BridgeShareRelayed.off;
    _noteRoster();
    notifyListeners();
  }

  void setReach(BridgeShareReach now) {
    reach = now;
    roster.value++;
    notifyListeners();
  }

  void setRelayed(BridgeShareRelayed now) {
    relayed = now;
    roster.value++;
    notifyListeners();
  }

  /// The link to send to whoever is joining, while this machine hosts.
  /// [address] is one more way in that only the person knows, such as a
  /// VPN's. It holds every way the engine knows of just now, so it is read
  /// again whenever [roster] moves.
  String? link({String? address}) {
    try {
      return _project?.shareLink(address: address);
    } catch (_) {
      return null;
    }
  }

  /// The secret of the invite as it stands, kept to share this project by
  /// the same invite next time.
  String? key() {
    try {
      return _project?.shareKey();
    } catch (_) {
      return null;
    }
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

  /// Look for a lost host by a new invite, with the host's [password] when
  /// it set one. False when [invite] is not one, or needs a password and has
  /// none.
  bool reinvite(String invite, {String? password}) {
    try {
      return _project?.shareReinvite(invite: invite, password: password) ??
          false;
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

  /// The property rows and the keyframes this person has in hand, by the
  /// names the panels' rows go by, what of them was last sent, and a count
  /// that moves on when either changes. The count is what [_send] compares,
  /// where the lists could be thousands long and it is asked for every frame
  /// of playback. Held as they are handed over and compared only in [_send],
  /// which is not reached unless the project is shared.
  List<String> _properties = const [];
  Set<String> _keys = const {};

  /// The keyframes each panel has selected, by the panel. Two panels have
  /// lanes with keyframes in them, and what the others see is both.
  final Map<String, Set<String>> _keysBy = {};
  List<String> _sentProperties = const [];
  Set<String> _sentKeys = const {};
  int _marks = 0;

  /// The composition open, the layers and the property rows selected in it
  /// and the playhead's frame. Does nothing unless the project is shared.
  void look(
      {required CompositionReference? comp,
      required List<LayerReference> layers,
      required int? playhead,
      List<String> properties = const []}) {
    _comp = comp;
    _layers = layers;
    _playhead = playhead;
    _properties = properties;
    _send();
  }

  /// The keyframes selected in the panel called [from], as its lanes name
  /// them. The set is kept, so it is one the caller will not change
  /// afterwards. Does nothing unless the project is shared.
  void keys(Set<String> keys, {String from = 'timeline'}) {
    _keysBy[from] = keys;
    _send();
  }

  /// The keyframes the others have selected in [comp], for a lane to ring:
  /// by the name the lanes give each, the colour of each person who has it.
  /// Empty when the project is not shared.
  Map<String, List<int>> keysInHand(CompositionReference? comp) {
    if (!active) return const {};
    final out = <String, List<int>>{};
    for (final person in inComp(comp)) {
      for (final key in person.keys) {
        (out[key] ??= []).add(person.colour);
      }
    }
    return out;
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
    if (!identical(_properties, _sentProperties) &&
        !listEquals(_properties, _sentProperties)) {
      _marks++;
    }
    _keys = _keysBy.length == 1
        ? _keysBy.values.first
        : {for (final held in _keysBy.values) ...held};
    if (!identical(_keys, _sentKeys) && !setEquals(_keys, _sentKeys)) {
      _marks++;
    }
    _sentProperties = _properties;
    _sentKeys = _keys;
    final now = '${_comp?.internalid}|${layers.map((l) => l.internallayerId)}'
        '|$_playhead|${_x?.round()}|${_y?.round()}|$_marks';
    if (now == _sent) return;
    _sent = now;
    try {
      project.sharePresence(
          comp: _comp,
          layers: layers,
          playhead: _playhead,
          cursorX: _x,
          cursorY: _y,
          properties: _comp == null ? const [] : _properties,
          keys: _comp == null ? const [] : _keys.toList());
    } catch (_) {
      // The project closed between the gesture and the call.
    }
  }
}
