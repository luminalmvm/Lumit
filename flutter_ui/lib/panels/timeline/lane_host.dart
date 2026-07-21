// The narrow seam a property row's keyframe lane talks to: the body owns the
// selection and the live drag, and each lane reports gestures back through this
// interface. Keeping it abstract lets the property-row widget stay ignorant of
// the body's state and lets tests drive a lane without the whole panel.

import 'lane_selection.dart';

/// The host (the Timeline body) a keyframe lane reports to. It owns the lane
/// selection and the in-flight drag; a lane reads them to draw and calls the
/// mutators on gestures.
abstract class TimelineLaneHost {
  /// The currently selected lane keys (drawn with an accent ring).
  Set<LaneKeyId> get selectedKeys;

  /// True while a lane key drag is in flight; selected keys draw offset by
  /// [keyDragDelta] frames for the live preview.
  bool get keyDragActive;

  /// The in-flight drag's frame delta (0 when no drag).
  int get keyDragDelta;

  /// A plain or additive click on a key selects it (egui `lane_select_click`).
  void keyTap(LaneKeyId key, {required bool additive});

  /// Begin a drag on [grabbed], grabbed at comp frame [grabFrame].
  void keyDragStart(LaneKeyId grabbed, int grabFrame);

  /// The pointer moved to comp [frame] — the host recomputes the live delta.
  void keyDragTo(int frame);

  /// Commit the drag (one `shiftKeyframes` per affected channel).
  void keyDragEnd();

  /// Remove a single key (right-click on it).
  void keyRemove(LaneKeyId key);
}
