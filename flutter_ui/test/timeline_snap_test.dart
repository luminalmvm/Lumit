// Timeline snapping arithmetic (docs/07-UI-SPEC.md §4.5).
//
// Pure, so checked here against hand-computed cases rather than by dragging in
// a widget tree — the same reasoning timeline_drag_test.dart follows for the
// row-height maths.

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_snap.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  group("A Sequence layer's cuts", () {
    // A row whose start offset is not its in point: it starts half a second
    // before the frame it first shows, and its one clip sits a second into the
    // row. The clip's box is therefore drawn at frame 90, and the row's own in
    // point is frame 60.
    BridgeLayerEntry rowWithAClip() {
      final id = UuidValue.fromString(const Uuid().v4());
      return BridgeLayerEntry(
        layer: LayerReference(
          internalprojectId: id,
          internalcompId: id,
          internallayerId: id,
        ),
        info: BridgeLayerInfo(
          collapseForced: false,
          volumeDb: const BridgeScalar.static_(0),
          pan: const BridgeScalar.static_(0),
          wired: false,
          textAnimators: const [],
          name: 'Music',
          kind: BridgeLayerKind.sequence,
          switches: const BridgeLayerSwitches(
            visible: true,
            audible: true,
            locked: false,
            solo: false,
            threeD: false,
            fx: true,
            motionBlur: false,
            collapse: false,
            shy: false,
            acceptsLights: true,
          ),
          blend: 0,
          span: const BridgeSpan(
            inPoint: BridgeRational(num: 1, den: 1),
            outPoint: BridgeRational(num: 3, den: 1),
            startOffset: BridgeRational(num: 1, den: 2),
          ),
          inFrame: 60,
          outFrame: 180,
          clipFrames: Int64List.fromList([90]),
          clips: [
            BridgeClip(
              id: id,
              placeStart: const BridgeRational(num: 1, den: 1),
              placeDuration: const BridgeRational(num: 1, den: 1),
              gainDb: 0,
              startFrame: 90,
              endFrame: 150,
              retimed: false,
              retime: const BridgeScalar.static_(0),
              fadeIn: const BridgeClipFade(
                  seconds: 0, shape: BridgeClipFadeShape.linear()),
              fadeOut: const BridgeClipFade(
                  seconds: 0, shape: BridgeClipFadeShape.linear()),
              effects: const [],
              fx: true,
              sourceName: 'Take 3.wav',
            ),
          ],
          transform: BridgeTransform(
            anchorX: const BridgeScalar.static_(0),
            anchorY: const BridgeScalar.static_(0),
            positionX: const BridgeScalar.static_(0),
            positionY: const BridgeScalar.static_(0),
            positionZ: const BridgeScalar.static_(0),
            scaleX: const BridgeScalar.static_(100),
            scaleY: const BridgeScalar.static_(100),
            rotation: const BridgeScalar.static_(0),
            rotationX: const BridgeScalar.static_(0),
            rotationY: const BridgeScalar.static_(0),
            opacity: const BridgeScalar.static_(100),
          ),
          axisModes: const BridgeAxisModes(
            anchor: BridgeAxisMode.combined,
            position: BridgeAxisMode.combined,
            scale: BridgeAxisMode.linked,
          ),
          effects: const [],
          styles: const [],
          label: 0,
          masks: const [],
          paint: const [],
          shapeContents: const [],
          markers: const [],
          flow: false,
          flowInputRate: const BridgeScalar.static_(0),
          trackCorrected: false,
        ),
      );
    }

    test('are the frames the clips are drawn at, offset and all', () {
      final targets = snapTargetsOf(
        layers: [rowWithAClip()],
        compMarkers: const [],
        keyRows: const [],
        playheadFrame: 0,
        work: (start: 0, end: 180, whole: true),
        fps: 60,
      );
      expect(
        targets
            .where((t) => t.kind == SnapKind.editPoint)
            .map((t) => t.frame)
            .toList(),
        [90.0],
        reason: "a row's in point is not its start offset",
      );
    });
  });

  group('What a drag lands on', () {
    // Ten pixels per frame: the default eight-pixel slop is therefore
    // 0.8 frames, which makes every case below easy to reason about.
    const perFrame = 10.0;

    test('a target within the slop takes the drag exactly', () {
      final r = snapFrame(
        frame: 12.37,
        targets: const [SnapTarget(12.5, SnapKind.marker)],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.frame, 12.5, reason: 'it lands ON the marker, not near it');
      expect(r.caught, const SnapTarget(12.5, SnapKind.marker));
    });

    test('the nearest target wins when several are in reach', () {
      final r = snapFrame(
        frame: 12.4,
        targets: const [
          SnapTarget(12.0, SnapKind.layerIn),
          SnapTarget(12.5, SnapKind.marker),
          SnapTarget(12.7, SnapKind.playhead),
        ],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.caught?.kind, SnapKind.marker);
      expect(r.frame, 12.5);
    });

    /// **The spec's rule that makes snapping feel right at every zoom**:
    /// distance is measured in screen pixels, never in time. The same target,
    /// the same distance in frames, is caught when zoomed out and missed when
    /// zoomed in.
    test('the reach is in pixels, so the magnification is the precision', () {
      const target = [SnapTarget(14, SnapKind.marker)];
      // Zoomed out: two frames away is 8 px at 4 px/frame... just outside.
      expect(
        snapFrame(
                frame: 12, targets: target, perFrame: 4.0, magnet: true)
            .caught,
        isNull,
      );
      // Zoomed further out: the same two frames is 6 px, and it catches.
      expect(
        snapFrame(frame: 12, targets: target, perFrame: 3.0, magnet: true)
            .caught,
        const SnapTarget(14, SnapKind.marker),
      );
      // Zoomed in: 100 px away, nowhere near.
      expect(
        snapFrame(frame: 12, targets: target, perFrame: 50.0, magnet: true)
            .caught,
        isNull,
      );
    });
  });

  /// **A place is offered once.** A baked After Effects camera keys all seven
  /// of its curves on every frame, so the panel gathered seventeen thousand
  /// keyframe targets for the two thousand frames they sit on — a list rebuilt
  /// with the panel and walked again on every pointer move of a drag.
  group('The keyframe places the gatherer offers', () {
    List<double> keyFrames(List<SnapKeyRow> rows, {String? exceptRow}) =>
        snapTargetsOf(
          layers: const [],
          compMarkers: const [],
          keyRows: rows,
          playheadFrame: 0,
          work: (start: 0, end: 100, whole: true),
          fps: 25,
          exceptRow: exceptRow,
        )
            .where((t) => t.kind == SnapKind.keyframe)
            .map((t) => t.frame)
            .toList();

    test('a frame the dragged lane shares with another is still offered', () {
      // Leaving the dragged row out may not take away a place another lane
      // also has: frame four stands because `b` is keyed there too.
      expect(
        keyFrames(const [
          (rowId: 'a', frames: [4.0, 9.0]),
          (rowId: 'b', frames: [4.0]),
        ], exceptRow: 'a'),
        [4.0],
      );
    });
  });
}