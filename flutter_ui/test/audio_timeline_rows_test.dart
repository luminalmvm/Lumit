// Which layers each timeline panel lists, and where a clip dragged about the
// Audio timeline lands (docs/impl/audio-timeline.md §5, plan 9): the Audio
// timeline keeps what can be heard and fades what is also a picture, the layer
// Timeline folds the Audio layers under the Sound mix row, and a drag names the
// track it crossed into. Pure, so the rules are checked here rather than by
// probing media in a widget tree, exactly as timeline_rows_test.dart checks the
// twirl's reach.

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_timeline_clips_frb.dart';
import 'package:lumit_flutter/panels/audio_timeline_rows_frb.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/timeline_bar_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_metrics_frb.dart';
import 'package:lumit_flutter/panels/volume_band_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// The gain line's scale (docs/impl/audio-timeline.md §5, plan 18): the
  /// Volume band's own mapping read from 0 dB down, so a clip at unity draws
  /// its line at the top of its box and a silent one at the foot.
  group('the gain line maps dB down the box', () {
    // A track two lane rows tall, and the box inset two pixels either side.
    const strip = 60.0;
    const box = strip - 4;

    test('a y read back is the dB it was drawn from', () {
      for (final db in [0.0, -6.0, -24.0, volumeBandFloorDb]) {
        expect(volumeBandDbOfY(volumeBandY(db, box, topDb: 0), box, topDb: 0),
            closeTo(db, 0.001));
      }
    });
  });

  /// A layer entry with only the fields the view rule reads filled in.
  BridgeLayerEntry entry(String name,
      {BridgeLayerKind kind = BridgeLayerKind.footage, bool audible = true}) {
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
        name: name,
        kind: kind,
        switches: BridgeLayerSwitches(
          visible: true,
          audible: audible,
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
          inPoint: BridgeRational(num: 0, den: 1),
          outPoint: BridgeRational(num: 1, den: 1),
          startOffset: BridgeRational(num: 0, den: 1),
        ),
        inFrame: 0,
        outFrame: 10,
        clipFrames: Int64List(0),
        clips: const [],
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

  String idOf(BridgeLayerEntry e) => e.layer.internallayerId.toString();

  // A stack of one of everything: a title, footage with picture and sound,
  // the same detached (muted picture over an Audio layer), a music track,
  // a muted music track, and a precomp that sounds.
  final title = entry('Title', kind: BridgeLayerKind.text);
  final clip = entry('Clip');
  final detached = entry('Detached clip', audible: false);
  final detachedSound =
      entry('Detached clip audio', kind: BridgeLayerKind.audio);
  final music = entry('Music', kind: BridgeLayerKind.audio);
  final mutedMusic =
      entry('Old take', kind: BridgeLayerKind.audio, audible: false);
  final nested = entry('Nested', kind: BridgeLayerKind.precomp);
  final stack = [
    title,
    clip,
    detached,
    detachedSound,
    music,
    mutedMusic,
    nested
  ];
  final hasAudio = {
    idOf(clip): true,
    idOf(detached): true,
    idOf(detachedSound): true,
    idOf(music): true,
    idOf(mutedMusic): true,
    idOf(nested): true,
    idOf(title): false,
  };
  final hasPicture = {
    idOf(clip): true,
    idOf(detached): true,
    idOf(detachedSound): false,
    idOf(music): false,
    idOf(mutedMusic): false,
    idOf(nested): true,
    idOf(title): true,
  };

  group('The Audio timeline\'s tracks', () {
    test('lists what can be heard, and fades what is also a picture', () {
      final view = timelineViewLayers(
          layers: stack,
          audioTimeline: true,
          mixOpen: false,
          hasAudio: hasAudio,
          hasPicture: hasPicture);
      expect(view.shown, [clip, detachedSound, music, mutedMusic, nested],
          reason: 'the title has no sound; the detached picture row is muted');
      expect(view.dimmed, {idOf(clip), idOf(nested)},
          reason: 'a picture row is read, not touched, until it is detached');
      expect(view.folded, isEmpty,
          reason: 'nothing folds in the Audio timeline');
    });
  });

  group('The Sound mix fold', () {
    test('takes the Audio layers out of the picture edit', () {
      final view = timelineViewLayers(
          layers: stack,
          audioTimeline: false,
          mixOpen: false,
          hasAudio: hasAudio,
          hasPicture: hasPicture);
      expect(view.shown, [title, clip, detached, nested],
          reason: 'sound-only rows fold; a picture row with sound stays');
      expect(view.folded, [detachedSound, music, mutedMusic]);
      expect(view.dimmed, isEmpty,
          reason: 'nothing dims in the layer Timeline');
    });
  });

  group('Where a dragged clip lands', () {
    // Three tracks, each two 20px lane rows tall and none of them open: the
    // bands are 0 to 40, 40 to 80 and 80 to 120, and the middle one is a faded
    // picture row.
    AudioTrackRow row(BridgeLayerEntry e, {bool dimmed = false}) =>
        AudioTrackRow(
          entry: e,
          id: idOf(e),
          open: false,
          dimmed: dimmed,
          foldRows: const [],
          rowHeight: 20,
        );
    final tracks = [row(music), row(clip, dimmed: true), row(mutedMusic)];

    test('names the track the pointer is over', () {
      expect(audioTrackAt(tracks, 0), 0);
      expect(audioTrackAt(tracks, 39), 0);
      expect(audioTrackAt(tracks, 80), 2);
      expect(audioTrackAt(tracks, 119), 2);
    });
  });

  /// A clip with only the fields the geometry reads filled in.
  BridgeClip clipAt(String id, int start, int end) => BridgeClip(
        id: UuidValue.fromString(id),
        placeStart: const BridgeRational(num: 0, den: 1),
        placeDuration: const BridgeRational(num: 1, den: 1),
        startFrame: start,
        endFrame: end,
        retimed: false,
        retime: const BridgeScalar.static_(0),
        fadeIn: const BridgeClipFade(
            seconds: 0, shape: BridgeClipFadeShape.linear()),
        fadeOut: const BridgeClipFade(
            seconds: 0, shape: BridgeClipFadeShape.linear()),
        effects: const [],
        fx: true,
        gainDb: 0,
        sourceName: 'Take.wav',
      );

  group('Which clip a press takes hold of', () {
    // A hundred frames over a thousand pixels, so a frame is ten wide and the
    // eight-pixel trim zone is most of one. Two clips laid across each other:
    // 0 to 40 with 30 to 70 over it, so the overlap runs from frame 30 to 40
    // and the earlier clip's tail zone is inside it.
    const axis = TimelineAxis(frames: 100, width: 1012);
    final early = clipAt('11111111-1111-1111-1111-111111111111', 0, 40);
    final later = clipAt('22222222-2222-2222-2222-222222222222', 30, 70);
    final clips = [early, later];

    test('an edge before a body, so a crossfade keeps its earlier tail', () {
      final hit = audioClipGrabAt(clips, axis, axis.xOf(40) - 3);
      expect(hit?.clip.id, early.id,
          reason: "the later clip's box covers the join, and took the press");
      expect(hit?.grab, BarGrab.trimOut);
    });
  });

  group("A clip's drop-down", () {
    final fxId = UuidValue.fromString(const Uuid().v4());
    final clipId = UuidValue.fromString(const Uuid().v4());
    final reverb = BridgeEffectInstanceInfo(
      id: fxId,
      name: 'clap:com.example.reverb',
      // Named, so the heading needs no trip to the catalogue for its label -
      // this test holds no engine.
      customName: 'Reverb',
      enabled: false,
      audio: true,
      values: const [],
      linkedPairs: const [],
      derivedParams: const [],
      hiddenRows: const [],
      disabledRows: const [],
      rowOptions: const [],
    );
    final sound = BridgeClip(
      id: clipId,
      placeStart: const BridgeRational(num: 0, den: 1),
      placeDuration: const BridgeRational(num: 1, den: 1),
      startFrame: 0,
      endFrame: 24,
      retimed: false,
      retime: const BridgeScalar.static_(0),
      fadeIn:
          const BridgeClipFade(seconds: 0, shape: BridgeClipFadeShape.linear()),
      fadeOut:
          const BridgeClipFade(seconds: 0, shape: BridgeClipFadeShape.linear()),
      effects: [reverb],
      fx: true,
      gainDb: 0,
      sourceName: 'Take 3.wav',
    );

    test('opens on to the clip name, then a heading per effect', () {
      final rows = clipFoldRows(clip: sound, open: {clipFoldPrefix(clipId)});
      expect(rows.length, 2);
      expect((rows.first as FoldGroupRow).label, 'Take 3.wav');
      expect(rows.first.depth, 1);
      final fx = rows.last as FoldGroupRow;
      expect(fx.depth, 2);
      expect(fx.open, isFalse, reason: 'each effect twirls for itself');
      expect(fx.enabled, isFalse,
          reason: 'the heading wears the effect\'s own bypass tick');
    });

    test('a path roots under the clip, which is no layer id', () {
      final path = foldRowPath(
        'some-layer',
        FoldEffectParamRow(
          reverb,
          const BridgeParamInfo(
            id: 'mix',
            label: 'Mix',
            kind: BridgeParamKind.action(),
            unit: BridgeUnit.raw,
            derived: false,
          ),
          null,
          depth: 3,
          clip: clipId,
        ),
      );
      expect(path, '${clipFoldPrefix(clipId)}/effects/$fxId/mix');
      expect(layerIdOfPath(path), clipFoldPrefix(clipId),
          reason: 'a clip prefix cannot be mistaken for a layer');
      expect(effectIdOfPath('${clipFoldPrefix(clipId)}/effects/$fxId'),
          fxId.toString());
      expect(
          isUnderPath('${clipFoldPrefix(clipId)}/effects/$fxId', path), isTrue);
    });
  });
}
