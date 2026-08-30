// One rule for what playback runs round.
//
// # In plain terms
//
// Press play and the picture runs to the end of the work area, then starts
// again from its beginning — that is the loop, and docs/07 §10 makes it the
// default mode. What was wrong with it was where the playhead is parked when
// play is pressed.
//
// Parked *before* the work area, playback previewed from where you stood and
// fell into the loop when it reached the end. Parked *after* it, the very
// first frame to arrive was already past the end, so the loop yanked the
// playhead back inside before you had seen anything — the tail of a comp could
// not be previewed at all. The two sides of the work area now behave the same:
// a run that starts past the end simply does not loop. It previews forward
// from where you parked, and stopping puts you back (K-254).

/// The span playback loops round for a run started at `playhead`, or null when
/// this run does not loop and plays out to the composition's end instead.
///
/// `workStart`/`workEnd` are the stored work area in frames, both null when the
/// comp has never been narrowed.
({int start, int end})? playbackLoop({
  required int? workStart,
  required int? workEnd,
  required int playhead,
}) {
  if (workStart == null || workEnd == null) return null;
  // A span with no room in it is not a loop — it would restart on every frame.
  if (workEnd <= workStart) return null;
  // Parked past the end: preview from there, do not snap back inside.
  if (playhead > workEnd) return null;
  return (start: workStart, end: workEnd);
}
