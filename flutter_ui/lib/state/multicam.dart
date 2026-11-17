// Multicam editing as the interface drives it: which halves a switch reaches,
// the multicam clip under the playhead, the page of angles the angle viewer
// shows, and the one job that makes or syncs a multicam comp.
//
// **In plain terms.** The engine decides every edit: cutting to an angle,
// switching one, stepping, painting a stretch, flattening. This holds the
// little the interface owns between those calls (the switch mode and the
// page), says what a key or a click means, and polls the sync job on the
// interface's tick, as an export is polled.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../src/rust/api/composition.dart';
import '../src/rust/api/cut.dart';
import '../src/rust/api/cut/multicam.dart';
import '../src/rust/api/layer.dart';
import '../src/rust/api/multicam.dart';
import '../src/rust/api/multicam/grid.dart';
import '../src/rust/api/multicam/make.dart';
import '../src/rust/api/project.dart';

/// What finished when the job landed: the comp it made or synced.
typedef MulticamLanded = void Function(UuidValue comp, {required bool made});

class MulticamState {
  MulticamState({
    required this.project,
    required this.comp,
    required this.playhead,
    required this.linked,
    required this.notice,
    required this.edited,
    required this.refused,
  });

  /// The project, the fronted comp and the playhead, read when asked.
  final ProjectReference? Function() project;
  final CompositionReference? Function() comp;
  final int Function() playhead;

  /// Whether edits carry linked clips along: the Cut timeline's Linked switch.
  final bool Function() linked;

  /// The status line, a document change, and a Cut edit's refusal.
  final void Function(String message, {bool error}) notice;
  final VoidCallback edited;
  final void Function(BridgeCutResult result) refused;

  /// Which halves of a multicam clip a cut or a switch reaches. The picture
  /// alone until changed, so the sound stays on the angle a clip starts on,
  /// a recorder where there is one.
  final ValueNotifier<BridgeAngleSwitch> mode =
      ValueNotifier(BridgeAngleSwitch.picture);

  /// The page of angles the angle viewer shows, counting from nought.
  final ValueNotifier<int> page = ValueNotifier(0);

  /// Whether the angle viewer draws each angle's own timecode.
  final ValueNotifier<bool> timecodes = ValueNotifier(false);

  /// The multicam clip under the playhead of the fronted comp, or null.
  /// Refreshed by [follow].
  final ValueNotifier<BridgeMulticamAt?> under = ValueNotifier(null);

  /// The angle a cut or switch last went to, and when: what the angle viewer
  /// flashes.
  final ValueNotifier<(UuidValue, int)?> lastCut = ValueNotifier(null);

  /// Counts the cuts to an angle. The Cut timeline lets its selection go at
  /// each: the pieces of a picked clip would all stay picked, and Angle on
  /// one of them would then switch every one.
  final ValueNotifier<int> divided = ValueNotifier(0);

  /// How far the running job has got, nought to one, or null with none.
  final ValueNotifier<double?> working = ValueNotifier(null);

  /// Called when a job lands with the comp it made or synced.
  MulticamLanded? onLanded;

  /// Whether a window is showing [working], so the status line need not.
  bool shown = false;

  Timer? _poll;

  /// Read which multicam clip is under `frame` of the fronted comp. Cheap: one
  /// engine read, nothing cloned.
  void follow([int? frame]) {
    final c = comp();
    if (c == null) {
      under.value = null;
      return;
    }
    try {
      final at = c.multicamAt(
          frame: frame ?? playhead(), timecodes: timecodes.value);
      if (at != under.value) under.value = at;
    } catch (_) {
      under.value = null;
    }
  }

  /// The angles of the multicam clip under the playhead on this page, in
  /// order: what the digits 1 to 9 reach.
  List<BridgeAngle> get pageAngles {
    final at = under.value;
    if (at == null) return const [];
    final angles = at.multicam.angles;
    final grid = angleGrid(angles: angles.length, page: page.value);
    final start = grid.first;
    final end = (start + grid.count).clamp(0, angles.length);
    return angles.sublist(start.clamp(0, angles.length), end);
  }

  /// A key the angle viewer or the Cut timeline was given. True when it was
  /// a multicam action and something was done; false for a digit with no
  /// multicam clip under the playhead, which then goes to its marker.
  bool handle(String action) {
    if (action.startsWith('multicam.cut.') ||
        action.startsWith('multicam.switch.')) {
      final cut = action.startsWith('multicam.cut.');
      final n = int.tryParse(action.split('.').last) ?? 0;
      follow();
      final angles = pageAngles;
      if (n < 1 || n > angles.length) return false;
      final angle = angles[n - 1].id;
      // Over footage an angle holds, a digit puts that angle above it.
      if (under.value?.plain == true) {
        if (!cut) return false;
        overwriteAbove(angle);
      } else {
        cut ? cutTo(angle) : switchTo(angle);
      }
      return true;
    }
    switch (action) {
      case 'multicam.angle.next' || 'multicam.angle.prev':
        step(forward: action.endsWith('next'));
        return true;
      case 'multicam.mode.both':
        mode.value = BridgeAngleSwitch.both;
        return true;
      case 'multicam.mode.picture':
        mode.value = BridgeAngleSwitch.picture;
        return true;
      case 'multicam.mode.sound':
        mode.value = BridgeAngleSwitch.sound;
        return true;
      case 'multicam.page.next' || 'multicam.page.prev':
        turnPage(action.endsWith('next') ? 1 : -1);
        return true;
    }
    return false;
  }

  /// Cut the multicam clip under the playhead there and switch the piece
  /// after the cut to [angle]. A sound angle is reached by the sound alone.
  void cutTo(UuidValue angle, {int? frame, LayerReference? row}) {
    final c = comp();
    if (c == null) return;
    final at = frame ?? playhead();
    _apply(
        () => c.cutToAngle(
            atFrame: at,
            angle: angle,
            switch_: _modeFor(angle),
            row: row,
            linked: linked()),
        angle,
        at,
        divides: true);
  }

  /// Switch the multicam clip under the playhead to [angle], whole, or the
  /// named clips when given. [mode] names the halves it reaches, else the
  /// switch mode does.
  void switchTo(UuidValue angle,
      {List<UuidValue> clips = const [], BridgeAngleSwitch? mode}) {
    final c = comp();
    if (c == null) return;
    final at = playhead();
    _apply(
        () => c.cutSwitchAngle(
            clips: clips,
            atFrame: at,
            angle: angle,
            switch_: mode ?? _modeFor(angle)),
        angle,
        at);
  }

  /// Switch the multicam clip under the playhead to the next angle, or the
  /// one before, round the angles.
  void step({required bool forward}) {
    final c = comp();
    if (c == null) return;
    try {
      _settle(c.cutStepAngle(
          atFrame: playhead(), forward: forward, switch_: mode.value));
    } catch (_) {}
  }

  /// Show [angle] from [from] to [to] on the multicam clip there.
  void paint(UuidValue angle, int from, int to) {
    final c = comp();
    if (c == null || to <= from) return;
    _apply(
        () => c.cutPaintAngle(
            fromFrame: from,
            toFrame: to,
            angle: angle,
            switch_: _modeFor(angle),
            linked: linked()),
        angle,
        from,
        divides: true);
  }

  /// Put the footage each named multicam clip's angle plays in its place.
  void flatten(List<UuidValue> clips) {
    final c = comp();
    if (c == null || clips.isEmpty) return;
    try {
      _settle(c.cutFlatten(clips: clips, linked: linked()));
    } catch (_) {}
  }

  /// Hear [angle] under the multicam clip under the playhead as well as the
  /// sound it plays, or stop hearing it when it already is.
  void hear(UuidValue angle) {
    final c = comp();
    final at = under.value;
    if (c == null || at == null || at.plain) return;
    try {
      _settle(c.cutHearAngle(
          clip: at.clip, angle: angle, on_: !at.also.contains(angle)));
    } catch (_) {}
  }

  /// Over footage one angle holds, put [angle] in step with it on the
  /// picture row above, from the playhead to the Out mark after it, else to
  /// the end of the footage's clip.
  void overwriteAbove(UuidValue angle) {
    final c = comp();
    final at = under.value;
    if (c == null || at == null || !at.plain) return;
    final from = playhead();
    // Not after the playhead, which the engine reads as the clip's end.
    var to = from;
    try {
      final marked = c.getWorkArea();
      if (marked != null) {
        final out = c.frameAtTime(time: marked.outPoint);
        if (out > from) to = out;
      }
    } catch (_) {}
    _apply(
        () => c.cutOverwriteAngle(atFrame: from, toFrame: to, angle: angle),
        angle,
        from);
  }

  /// Show the next page of angles, or the one before.
  /// [angles] is how many the angle viewer shows when it stands on its own,
  /// with no clip under the playhead to count.
  void turnPage(int by, {int? angles}) {
    final count = angles ?? under.value?.multicam.angles.length ?? 0;
    final pages = angleGrid(angles: count, page: 0).pages;
    page.value = (page.value + by).clamp(0, pages - 1);
  }

  /// A sound angle has no picture, so only its sound can be switched to it.
  BridgeAngleSwitch _modeFor(UuidValue angle) {
    final at = under.value;
    final sound = at?.multicam.angles.any((a) => a.id == angle && a.sound);
    return sound == true ? BridgeAngleSwitch.sound : mode.value;
  }

  void _apply(BridgeCutResult Function() edit, UuidValue angle, int frame,
      {bool divides = false}) {
    try {
      final result = edit();
      if (result == BridgeCutResult.done) {
        lastCut.value = (angle, frame);
        if (divides) divided.value++;
      }
      _settle(result);
    } catch (_) {}
  }

  void _settle(BridgeCutResult result) {
    if (result == BridgeCutResult.done) {
      edited();
      follow();
    } else if (result != BridgeCutResult.nothing) {
      refused(result);
    }
  }

  /// Make a multicam comp of the footage [items]. Its progress shows in
  /// [working] and it lands through [onLanded].
  bool make(List<UuidValue> items, BridgeMulticamNew settings) {
    final p = project();
    if (p == null) return false;
    try {
      p.multicamNew(items: items, settings: settings);
    } catch (_) {
      return false;
    }
    _watch(p, made: true);
    return true;
  }

  /// Line [layer]'s files up again by their sound against [to]'s, inside the
  /// multicam comp [mc].
  bool syncAngle(CompositionReference mc, UuidValue layer, UuidValue to) {
    final p = project();
    if (p == null) return false;
    try {
      mc.multicamSyncAngle(layer: layer, to: to);
    } catch (_) {
      return false;
    }
    _watch(p, made: false);
    return true;
  }

  /// Stop the running job. Nothing is made or moved.
  void cancel() {
    final p = project();
    _poll?.cancel();
    _poll = null;
    working.value = null;
    try {
      p?.multicamCancel();
    } catch (_) {}
  }

  void _watch(ProjectReference p, {required bool made}) {
    _poll?.cancel();
    working.value = 0;
    _poll = Timer.periodic(const Duration(milliseconds: 100), (timer) {
      final BridgeMulticamJob job;
      try {
        job = p.multicamPoll();
      } catch (_) {
        timer.cancel();
        working.value = null;
        return;
      }
      switch (job) {
        case BridgeMulticamJob_Working(:final done):
          working.value = done;
        case BridgeMulticamJob_Done(
            :final comp,
            :final uncertain,
            :final unsynced
          ):
          timer.cancel();
          working.value = null;
          edited();
          if (unsynced.isNotEmpty) {
            notice(l10n.multicamUnsynced(
                unsynced.length, unsynced.join(', ')));
          }
          if (uncertain.isNotEmpty) {
            notice(l10n.multicamUncertain(
                uncertain.length, uncertain.join(', ')));
          }
          onLanded?.call(comp, made: made);
          follow();
        case BridgeMulticamJob_Failed():
          timer.cancel();
          working.value = null;
          notice(l10n.multicamFailed, error: true);
        case BridgeMulticamJob_Idle():
          timer.cancel();
          working.value = null;
      }
    });
  }

  void dispose() {
    _poll?.cancel();
    mode.dispose();
    page.dispose();
    timecodes.dispose();
    under.dispose();
    lastCut.dispose();
    divided.dispose();
    working.dispose();
  }
}
