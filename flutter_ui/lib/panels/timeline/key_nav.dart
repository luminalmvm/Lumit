// The keyframe-navigator logic (◄ ◆ ►), ported from the egui
// `key_nav_targets` (crates/lumit-ui/src/shell/inspector/keyframe_nav.rs). All
// times here are comp frames (the bridge reports keyframe frames in comp-frame
// space and the playhead is a comp frame too), so the half-frame tolerance
// collapses to integer equality — a key "at the playhead" is one on the exact
// frame. Pure so the prev/on-key/next resolution is unit-tested.

/// What a navigator can do from the [playhead] over the sorted key [frames]:
/// the previous key frame (or null), whether a key sits on the playhead, and
/// the next key frame (or null).
class KeyNavTargets {
  final int? prev;
  final bool onKey;
  final int? next;

  const KeyNavTargets(this.prev, this.onKey, this.next);
}

/// Resolve the navigator targets: `prev` is the last key strictly before the
/// playhead, `next` the first strictly after, `onKey` true when a key lands on
/// it — mirroring egui's `key_nav_targets` with a half-frame tolerance.
KeyNavTargets keyNavTargets(List<int> frames, int playhead) {
  int? prev;
  int? next;
  var onKey = false;
  for (final f in frames) {
    if (f == playhead) onKey = true;
    if (f < playhead) {
      if (prev == null || f > prev) prev = f;
    } else if (f > playhead) {
      if (next == null || f < next) next = f;
    }
  }
  return KeyNavTargets(prev, onKey, next);
}
