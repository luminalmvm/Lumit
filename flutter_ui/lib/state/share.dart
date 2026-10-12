// A shared project as the interface sees it: which end this is, who else is
// here and what they are looking at, and whether the host can be reached.
// LumitState owns one and drives it. Panels listen and draw.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show StringCharacters;
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart' show lumitVersion;
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';
import 'package:lumit_flutter/state/reviews.dart' show commenterColour;
import 'package:lumit_flutter/state/updates.dart' show versionFromBootLine;

import 'preview_throttle.dart';

/// Why a guest was turned away or sharing ended, as a sentence to show.
String shareEndingText(BridgeShareEnding reason) => switch (reason) {
      BridgeShareEnding_Closed() => l10n.shareEndedClosed,
      BridgeShareEnding_VersionMismatch(:final host) =>
        l10n.shareVersionMismatch(host),
      BridgeShareEnding_Full() => l10n.shareFull,
      BridgeShareEnding_Unsafe() => l10n.shareUnsafe,
      BridgeShareEnding_Removed() => l10n.shareRemoved,
    };

/// What others did in a project kept online while this machine was not
/// there, in a handful of words: [names] in the order their edits were
/// found, how many [edits] between them, and the [comps] those were in.
String shareActivityText(List<String> names, int edits, Set<String> comps) =>
    switch (names) {
      [final one] when comps.length == 1 =>
        l10n.shareActivityOneIn(one, edits, comps.first),
      [final one] => l10n.shareActivityOne(one, edits),
      [final one, final other] => l10n.shareActivityTwo(one, other, edits),
      _ => l10n.shareActivityMany(names.first, names.length - 1, edits),
    };

/// The version of this Lumit as it is written, such as 0.7.0.
String shareThisLumit() => versionFromBootLine(lumitVersion()) ?? '';

/// Why moving a project kept online forward to this Lumit did not happen, in
/// words. [version] is the Lumit that keeps it. Null when it did.
String? shareForwardText(BridgeOnlineForward outcome, String version) =>
    switch (outcome) {
      BridgeOnlineForward_Moved() => null,
      BridgeOnlineForward_Unreadable() => l10n.shareForwardUnreadable(version),
      BridgeOnlineForward_NotHost() => l10n.shareForwardNotHost,
      BridgeOnlineForward_Update(:final version) =>
        l10n.shareVersionsUpdate(version),
      BridgeOnlineForward_Refused(code: 'pro_required') =>
        l10n.shareOnlineErrorPro,
      BridgeOnlineForward_Refused() => l10n.shareOnlineErrorRefused,
      BridgeOnlineForward_Unreachable() => l10n.signInErrorOffline,
      BridgeOnlineForward_Failed() => l10n.shareForwardBusy(version),
    };

/// What joining by an invite and moving a project forward answer, set by
/// tests to stand in for another version of Lumit: two cannot be run side by
/// side.
BridgeJoinOutcome? Function(String invite)? shareJoinOverride;
BridgeOnlineForward Function()? shareForwardOverride;

/// The letters a person's disc carries: their initial, and a second letter
/// when someone else among [here] has the same initial, so Ana and Alex are
/// An and Al and not both A. Two people of one name stay one letter each.
String personLetters(String name, Iterable<String> here) {
  final own = name.trim();
  final letters = own.characters.where((c) => c.trim().isNotEmpty);
  final first = letters.firstOrNull?.toUpperCase() ?? '';
  for (final other in here) {
    final theirs = other.trim();
    if (theirs != own &&
        theirs.characters.firstOrNull?.toUpperCase() == first) {
      return '$first${letters.skip(1).firstOrNull?.toLowerCase() ?? ''}';
    }
  }
  return first;
}

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
            '|${Object.hashAll(p.properties)}|${Object.hashAll(p.keys)}'
            '|${Object.hashAll(p.changing)}')
        .join(';');
    if (now == _rosterWas) return;
    _rosterWas = now;
    roster.value++;
  }

  /// This guest has lost its host and is working on alone until it is back.
  /// For a project kept online it is Lumit's server that is lost.
  bool away = false;

  /// What a project kept online is to this machine, as the engine last said:
  /// whether this is its host, and what the interface shows Lumit's server
  /// for it. Null for a project that is not kept online.
  BridgeShareOnline? online;

  /// Lumit's server is taking edits to a project kept online. It stops when
  /// the host's Lumit Pro ends, and edits then wait on this machine.
  bool writable = true;

  /// Who changed what in a project kept online, newest sitting first, as the
  /// engine last listed it. [LumitState] reads it, at most twice a second
  /// and never in a build.
  List<BridgeShareSitting> activity = const [];

  /// The notes people have left in a project kept online, newest first, with
  /// this person's own that are still waiting to be sent before them. Read
  /// with [activity].
  List<BridgeShareNote> notes = const [];

  /// The letters [name]'s disc carries wherever it is drawn, so one person
  /// is the same mark in the strip, the Project panel and the window. Told
  /// apart from everyone in the project now and everyone in [activity] and
  /// [notes].
  String lettersOf(String name) => personLetters(name, [
        for (final person in people) person.name,
        for (final sitting in activity) sitting.name,
        for (final note in notes) note.name,
      ]);

  /// The colour someone who wrote or edited is drawn in: the person's own,
  /// while they are here by that name, and otherwise one that is always the
  /// same for the name.
  int colourOf(String name, {required bool mine}) {
    for (final person in people) {
      if (mine ? person.me : person.name == name) return person.colour;
    }
    return commenterColour(name);
  }

  /// Moves on each time [activity] is read again.
  final ValueNotifier<int> activityRead = ValueNotifier(0);

  /// Opens the Shared project window on what happened while this machine
  /// was away. Null until there is a window to open it in.
  VoidCallback? showActivity;

  void setActivity(List<BridgeShareSitting> now,
      [List<BridgeShareNote> left = const []]) {
    activity = now;
    notes = left;
    activityRead.value++;
  }

  /// How this machine stands to a project another version of Lumit keeps
  /// online, or null when nothing is in the way. The machine is away from
  /// the project for as long as this is set.
  BridgeShareVersions? versions;

  /// This machine is not in step with the project, whatever the reason: it
  /// is being moved forward, another Lumit keeps it, the server takes no
  /// edits, or the server or the host is out of reach. The other people are
  /// then only what was last heard of them, and are drawn fainter wherever
  /// they are marked. True when the window has a state to name, but for a
  /// project that is only nearly too large, which is still in step.
  bool get outOfStep =>
      forwarding || versions != null || (online != null && !writable) || away;

  /// The Lumit that keeps the project, when it is not this one.
  String? get versionsOf => switch (versions) {
        BridgeShareVersions_Update(:final version) ||
        BridgeShareVersions_Movable(:final version) ||
        BridgeShareVersions_HostFirst(:final version) =>
          version,
        null => null,
      };

  /// The project is being moved forward to this Lumit just now.
  bool forwarding = false;

  /// How many of this person's changes were dropped the last time the
  /// machine came back, until they have been told and say so, and what the
  /// first of them were to, twenty at most.
  int dropped = 0;
  List<BridgeShareDropped> droppedWhat = const [];

  /// This machine's own project from just before a return that held or
  /// dropped any of its edits, as the files the engine keeps, each with when
  /// it was written and what its own return dropped. Two at most: the
  /// latest such return first, and after it the earliest one not yet dealt
  /// with, which alone holds what that return dropped. Read when sharing
  /// begins and on every return, so they are known after Lumit is opened
  /// again too. A copy that could not be written is simply not here.
  List<BridgeShareReturnCopy> returnCopies = const [];

  void readReturnCopies() {
    try {
      returnCopies = _project?.shareReturnCopies() ?? const [];
    } catch (_) {
      returnCopies = const [];
    }
  }

  /// The person has done with what the returns dropped: the count goes,
  /// and every copy the engine kept with it.
  void forgetReturn() {
    try {
      _project?.shareReturnCopyForget();
    } catch (_) {
      // The project closed between the press and the call.
    }
    returnCopies = const [];
    setDropped(0);
  }

  /// Asks the host whether to move the project forward, and offers the
  /// update a project on a newer Lumit needs. Null until there is a window
  /// to do either in.
  VoidCallback? showForward, showUpdate;

  /// Opens the window that settles conflicts, and the Shared project
  /// window itself.
  VoidCallback? showConflicts, showWindow;

  void setVersions(BridgeShareVersions? now) {
    versions = now;
    roster.value++;
    notifyListeners();
  }

  void setForwarding(bool now) {
    forwarding = now;
    roster.value++;
    notifyListeners();
  }

  void setDropped(int now, [List<BridgeShareDropped> what = const []]) {
    dropped = now;
    droppedWhat = what;
    roster.value++;
    activityRead.value++;
    notifyListeners();
  }

  /// Read [online] again. Whose project it is and the invite it goes by are
  /// only known once the server has been found, and both can change there.
  void _readOnline() {
    try {
      online = _project?.shareOnlineState();
      versions = _project?.shareOnlineVersions();
    } catch (_) {
      online = null;
      versions = null;
    }
    final found = online;
    if (found != null) role = found.host ? ShareRole.host : ShareRole.guest;
  }

  /// For a host: whether people outside its network can get in.
  BridgeShareReach reach = const BridgeShareReach.off();

  /// For a host: whether it has a room at a relay.
  BridgeShareRelayed relayed = BridgeShareRelayed.off;

  /// How many conflicts a merge has left waiting to be chosen between.
  int held = 0;

  /// Moves on whenever anything about the project's footage changes: who has
  /// which file, a transfer, or an export being fetched for or done by
  /// someone else. Whoever draws any of that reads it again.
  final ValueNotifier<int> footage = ValueNotifier(0);

  /// Exports other people have asked this computer to do and that the
  /// person has not answered: the job, who asked, and the composition.
  final List<({String job, int from, String comp})> exportAsks = [];

  void footageChanged() {
    _readTransfers();
    footage.value++;
  }

  void exportAsked(String job, int from, String comp) {
    exportAsks.add((job: job, from: from, comp: comp));
    notifyListeners();
  }

  /// What the person numbered [id] is called, or null when they have gone.
  String? nameOf(int id) {
    for (final person in people) {
      if (person.id == id) return person.name;
    }
    return null;
  }

  /// Every footage transfer in flight, to and from this computer.
  /// As they stood when the footage last changed. Read there and held, so a
  /// window that lists them asks the engine nothing as it is drawn.
  List<BridgeShareTransfer> transfers() => _transfers;

  List<BridgeShareTransfer> _transfers = const [];

  void _readTransfers() {
    try {
      _transfers = _project?.shareTransfers() ?? const [];
    } catch (_) {
      _transfers = const [];
    }
  }

  /// How fetching what an export lacks is getting on.
  BridgeShareFetching fetching() {
    try {
      return _project?.shareFetching() ?? const BridgeShareFetching.idle();
    } catch (_) {
      return const BridgeShareFetching.idle();
    }
  }

  void cancelFetch() {
    try {
      _project?.shareFetchCancel();
    } catch (_) {
      // The project closed, which gave it up too.
    }
  }

  /// How the export asked of someone else's computer is getting on.
  BridgeShareAsking asking() {
    try {
      return _project?.shareAsking() ?? const BridgeShareAsking.idle();
    } catch (_) {
      return const BridgeShareAsking.idle();
    }
  }

  /// Answer an export someone asked this computer to do.
  void answerExport(String job, {required bool yes}) {
    exportAsks.removeWhere((ask) => ask.job == job);
    try {
      _project?.shareAnswerExport(job: job, yes: yes);
    } catch (_) {
      // The project closed between the question and the answer.
    }
  }

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

  /// The controls the others are changing right now, by the names their
  /// rows go by, and for each who has it: what a row is held for them by. A
  /// name leads with its layer, so it is the same row whichever composition
  /// either machine has open.
  Map<String, ({int colour, String name})> get changing => {
        for (final person in others)
          for (final name in person.changing)
            name: (colour: person.colour, name: person.name),
      };

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
    _readOnline();
    readReturnCopies();
    away = online?.away ?? false;
    // From here on it is the engine's word, each time it says.
    writable = online?.writable ?? true;
    held = 0;
    reach = as == ShareRole.host
        ? project.shareReach()
        : const BridgeShareReach.off();
    relayed = as == ShareRole.host
        ? project.shareRelayed()
        : BridgeShareRelayed.off;
    _sent = null;
    _readTransfers();
    _noteRoster();
    // A guest's copy opened again brings the conflicts it was closed with.
    held = project.shareConflicts().length;
    // Sharing has begun, which whoever follows the roster has to hear even
    // when nobody is on it yet.
    roster.value++;
    notifyListeners();
    _send();
  }

  /// Sharing is over, or the project it was for has gone.
  void end() {
    if (!active) return;
    _pointing.cancel();
    role = ShareRole.none;
    port = null;
    _project = null;
    people = const [];
    away = false;
    replaced = false;
    online = null;
    versions = null;
    forwarding = false;
    dropped = 0;
    droppedWhat = const [];
    returnCopies = const [];
    writable = true;
    setActivity(const []);
    held = 0;
    reach = const BridgeShareReach.off();
    relayed = BridgeShareRelayed.off;
    exportAsks.clear();
    _transfers = const [];
    footage.value++;
    _noteRoster();
    roster.value++;
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

  /// The host made a new invite while this machine was away, and it was not
  /// the one taken out. The copy is still a member and asks the server no
  /// more, until it is given the new invite.
  bool replaced = false;

  void setReplaced() {
    replaced = true;
    roster.value++;
    notifyListeners();
  }

  void setAway(bool now, {int? conflicts}) {
    away = now;
    if (!now) replaced = false;
    if (conflicts != null) held = conflicts;
    if (online != null) _readOnline();
    roster.value++;
    notifyListeners();
  }

  /// The server said something of the project that [online] holds: whether
  /// the invite to look is on, or that the project has grown too large.
  void onlineSaid() {
    _readOnline();
    roster.value++;
    notifyListeners();
  }

  void setWritable(bool now) {
    writable = now;
    roster.value++;
    notifyListeners();
    // Back with the host, which has forgotten what this person was looking
    // at: said again, though nothing here has changed since it was last said.
    if (!now) {
      _sent = null;
      _send();
    }
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

  /// The controls this person is dragging or typing in right now, by the
  /// names their rows go by. One or two at most.
  List<String> _changing = const [];

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
      List<String> properties = const [],
      List<String> changing = const []}) {
    _comp = comp;
    _layers = layers;
    _playhead = playhead;
    _properties = properties;
    // Someone who can only look changes nothing.
    _changing = online?.look ?? false ? const [] : changing;
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
    // A pointer crossing the Viewer is a new place for every pixel, and each
    // telling carries everything else this person has in hand as well. The
    // first goes at once and the newest of the rest a little later.
    if (_project != null) _pointing.request(_send);
  }

  final PreviewThrottle _pointing =
      PreviewThrottle(interval: const Duration(milliseconds: 50));

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
        '|$_playhead|${_x?.round()}|${_y?.round()}|$_marks|$_changing';
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
          keys: _comp == null ? const [] : _keys.toList(),
          // Said whichever composition is open: a name says whose it is.
          changing: _changing);
    } catch (_) {
      // The project closed between the gesture and the call.
    }
  }
}
