// Which layers each timeline panel lists, and where a clip dragged about the
// Audio timeline lands (docs/impl/audio-timeline.md §5, plan 9): the Audio
// timeline keeps what can be heard and fades what is also a picture, the layer
// Timeline folds the Audio layers under the Sound mix row, and a drag names the
// track it crossed into. Pure, so the rules are checked here rather than by
// probing media in a widget tree, exactly as timeline_rows_test.dart checks the
// twirl's reach.

import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_timeline_clips_frb.dart';
import 'package:lumit_flutter/panels/audio_timeline_fades_frb.dart';
import 'package:lumit_flutter/panels/audio_timeline_rows_frb.dart';
import 'package:lumit_flutter/panels/clip_fades.dart';
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

    test('0 dB is the top of the box and the floor is its foot', () {
      expect(volumeBandY(0, box, topDb: 0), closeTo(1, 0.001));
      expect(volumeBandY(volumeBandFloorDb, box, topDb: 0),
          closeTo(box - 1, 0.001));
      expect(volumeBandY(6, box, topDb: 0), closeTo(1, 0.001),
          reason: 'there is no room above unity for a boost to be drawn in');
    });

    test('a y read back is the dB it was drawn from', () {
      for (final db in [0.0, -6.0, -24.0, volumeBandFloorDb]) {
        expect(volumeBandDbOfY(volumeBandY(db, box, topDb: 0), box, topDb: 0),
            closeTo(db, 0.001));
      }
    });

    test('the line lies inside the box the strip draws', () {
      expect(audioClipGainY(0, strip), closeTo(3, 0.001),
          reason: 'a clip at unity is drawn as loud as the box is tall');
      expect(
          audioClipGainY(volumeBandFloorDb, strip), closeTo(strip - 3, 0.001));
      expect(audioClipGainY(-30, strip), greaterThan(audioClipGainY(-6, strip)),
          reason: 'quieter is further down, which is what the drag reads');
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

    test('keeps a muted Audio layer: mute is a mixing decision', () {
      expect(audioTimelineShows(mutedMusic, hasAudio: true, hasPicture: false),
          isTrue);
      expect(audioTimelineShows(detached, hasAudio: true, hasPicture: true),
          isFalse,
          reason: 'a muted picture row has given its sound to another row');
    });

    test('an unprobed layer is taken to have a picture and no sound', () {
      final view = timelineViewLayers(
          layers: [music],
          audioTimeline: true,
          mixOpen: false,
          hasAudio: const {},
          hasPicture: const {});
      expect(view.shown, isEmpty,
          reason: 'until the probe answers there is nothing to list');
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

    test('twirled open, the Audio layers stand in place', () {
      final view = timelineViewLayers(
          layers: stack,
          audioTimeline: false,
          mixOpen: true,
          hasAudio: hasAudio,
          hasPicture: hasPicture);
      expect(view.shown, stack, reason: 'stack order, nothing moved');
      expect(view.folded, [detachedSound, music, mutedMusic],
          reason: 'the row still says how many it holds');
    });

    test('a comp with no Audio layer has nothing to fold', () {
      final view = timelineViewLayers(
          layers: [title, clip],
          audioTimeline: false,
          mixOpen: false,
          hasAudio: hasAudio,
          hasPicture: hasPicture);
      expect(view.folded, isEmpty);
      expect(view.shown, [title, clip]);
    });
  });

  test('a dimmed row fades and goes deaf; an ordinary one is untouched', () {
    const child = SizedBox();
    expect(dimmedIf(false, child), same(child));
    final dimmed = dimmedIf(true, child);
    expect(dimmed, isA<IgnorePointer>());
    expect((dimmed as IgnorePointer).child, isA<Opacity>());
    expect(((dimmed).child! as Opacity).opacity, dimmedRowOpacity);
  });

  /// A track's height is its own (docs/impl/audio-timeline.md §5, plan 20):
  /// a band of lane rows the row's bottom edge sets, and the twirl's rows
  /// under it whatever that band is.
  group("A track's height", () {
    AudioTrackRow tall(int rows, {List<LayerFoldRow> fold = const []}) =>
        AudioTrackRow(
          entry: music,
          id: idOf(music),
          open: fold.isNotEmpty,
          dimmed: false,
          foldRows: fold,
          rowHeight: 20,
          rows: rows,
        );

    test('the band is the lane rows it was given', () {
      expect(tall(2).laneHeight, 40);
      expect(tall(4).laneHeight, 80);
    });

    test('a shut track is its band and nothing more', () {
      expect(tall(2).height, 40);
      expect(tall(4).height, 80);
    });

    test('an open one adds a lane row a row, whatever the band', () {
      final fold = [
        FoldVolumeRow(scalar: const BridgeScalar.static_(0), depth: 1),
        FoldGroupRow(path: 'effects', label: 'Effects', open: false, depth: 1),
      ];
      expect(tall(2, fold: fold).height, 80);
      expect(tall(4, fold: fold).height, 120,
          reason: 'the twirl costs the same two rows on a taller track');
    });

    test('two lane rows unless it is told otherwise', () {
      expect(
          tall(audioTrackMinRows).height,
          AudioTrackRow(
            entry: music,
            id: idOf(music),
            open: false,
            dimmed: false,
            foldRows: const [],
            rowHeight: 20,
          ).height,
          reason: 'a track opens at the height the board draws');
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

    test('a faded picture row is no landing place', () {
      expect(audioTrackAt(tracks, 60), isNull,
          reason: 'a row that takes no pointer takes no clip either');
    });

    test('there is no track above the first or below the last', () {
      expect(audioTrackAt(tracks, -1), isNull);
      expect(audioTrackAt(tracks, 200), isNull);
    });

    test('below the last track is where a clip gets a track of its own', () {
      expect(audioBelowTracks(tracks, 120), isTrue);
      expect(audioBelowTracks(tracks, 119), isFalse);
      expect(audioBelowTracks(tracks, -1), isFalse,
          reason: 'above the table is not below it');
    });

    test('an open track is taller, and the bands below it move down', () {
      final open = [
        AudioTrackRow(
          entry: music,
          id: idOf(music),
          open: true,
          dimmed: false,
          foldRows: [
            FoldVolumeRow(scalar: const BridgeScalar.static_(0), depth: 1)
          ],
          rowHeight: 20,
        ),
        row(mutedMusic),
      ];
      expect(audioTrackAt(open, 50), 0,
          reason: 'the twirl row is still track 0');
      expect(audioTrackAt(open, 70), 1);
    });
  });

  group('Which rows can hold a clip', () {
    test('a row of sound can, converted or not', () {
      expect(audioTrackTakesClips(music.info), isTrue);
      expect(audioTrackTakesClips(clip.info), isTrue,
          reason: 'footage becomes the one clip it has always been');
    });

    test('a precomp cannot: there is nothing to convert', () {
      expect(audioTrackTakesClips(nested.info), isFalse);
    });
  });

  group("A clip's trim zone", () {
    test('is the bar grab: eight pixels on a clip wide enough for them', () {
      expect(barGrabAt(7, 200), BarGrab.trimIn);
      expect(barGrabAt(9, 200), BarGrab.move);
      expect(barGrabAt(193, 200), BarGrab.trimOut);
    });

    test('is capped at a third, so a short clip keeps a body to hold', () {
      expect(barGrabAt(1, 12), BarGrab.trimIn);
      expect(barGrabAt(6, 12), BarGrab.move);
      expect(barGrabAt(11, 12), BarGrab.trimOut);
    });
  });

  group("A clip picture's origin", () {
    test('travels with the start edge, so none of the box is left bare', () {
      expect(
        audioClipOrigin(
            placeStartSeconds: 2, shiftFrames: 30, fps: 60, trimIn: true),
        2.5,
        reason: 'the buckets are asked for and drawn from the same second',
      );
    });

    test('holds while the whole clip slides: the picture rides along', () {
      expect(
        audioClipOrigin(
            placeStartSeconds: 2, shiftFrames: 30, fps: 60, trimIn: false),
        2.0,
      );
    });
  });

  group("The source's start", () {
    test('is where the sound begins, ahead of a head dragged out past it', () {
      // A clip at frame 120 with half a second of silence in front of it, at
      // sixty frames a second: the reach the engine reports starts thirty
      // frames later than the box does.
      expect(
        audioSourceStartFrame(startFrame: 120, reachStartFrame: 150),
        150,
        reason: 'the head snaps to it and the box wears the mark there',
      );
    });

    test('is nothing on a clip trimmed the ordinary way', () {
      expect(audioSourceStartFrame(startFrame: 120, reachStartFrame: 120), null,
          reason: 'the sound starts at the head: no silence to mark');
      expect(audioSourceStartFrame(startFrame: 120, reachStartFrame: 90), null,
          reason: 'trimmed in, so the source starts before the head');
    });

    test('is nothing when the engine reports no reach at all', () {
      expect(
        audioSourceStartFrame(startFrame: 120, reachStartFrame: null),
        null,
        reason: 'a retimed clip, or a source whose length would not read',
      );
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

    test('the body of the topmost clip, everywhere else in the overlap', () {
      final hit = audioClipGrabAt(clips, axis, axis.xOf(35));
      expect(hit?.clip.id, later.id);
      expect(hit?.grab, BarGrab.move);
    });

    test('nothing at all on empty ground', () {
      expect(audioClipGrabAt(clips, axis, axis.xOf(80)), isNull);
      expect(audioClipGrabAt(const [], axis, axis.xOf(10)), isNull);
    });
  });

  group('The ramps a drag carries', () {
    const shape = BridgeClipFadeShape.linear();
    final ramps = <ClipFadeRamp>[
      (clip: 'a', into: true, from: 0, to: 5, shape: shape),
      (clip: 'a', into: false, from: 35, to: 40, shape: shape),
      (clip: 'b', into: true, from: 60, to: 64, shape: shape),
    ];

    test('a move carries both of the dragged clip\'s ends', () {
      final moved = shiftRamps(ramps, 'a', BarGrab.move, 4);
      expect(moved.map((r) => (r.from, r.to)),
          [(4.0, 9.0), (39.0, 44.0), (60.0, 64.0)],
          reason: 'a clip nobody has hold of stays where it is');
    });

    test('a head trim carries the rising ramp alone', () {
      final moved = shiftRamps(ramps, 'a', BarGrab.trimIn, -3);
      expect(moved.map((r) => (r.from, r.to)),
          [(-3.0, 2.0), (35.0, 40.0), (60.0, 64.0)]);
    });

    test('a tail trim carries the falling one', () {
      final moved = shiftRamps(ramps, 'a', BarGrab.trimOut, 6);
      expect(moved.map((r) => (r.from, r.to)),
          [(0.0, 5.0), (41.0, 46.0), (60.0, 64.0)]);
    });

    test('a drag that has moved no frames changes nothing', () {
      expect(shiftRamps(ramps, 'a', BarGrab.move, 0), same(ramps));
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

    test('shows nothing until the clip is twirled open', () {
      expect(clipFoldRows(clip: sound, open: const {}), isEmpty);
    });

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
