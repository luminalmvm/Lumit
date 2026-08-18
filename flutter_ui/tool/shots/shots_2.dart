// Manual screenshots, sweep 2: the Timeline family.
//
// timeline · timeline-outline · timeline-lanes · layer-switches · transform ·
// keyframes-lane · keyframes-stopwatch · blend-modes · markers-ruler ·
// waveform · cache-bar
//
// The project is staged through the real engine and then worked the way a
// person would — the twirls are clicked, the dropdown is opened, the splitter
// is moved when a fold needs the room — because a panel's fold state is
// deliberately its own and a screenshot of a state the program cannot be put
// into is not a screenshot of the program.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_2.dart
//
// A first pass with `LUMIT_SHOTS_NOCROP=1` and `LUMIT_SHOTS_OUT` set writes
// whole windows somewhere harmless, for checking what the crops are aimed at.

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import 'shots_common.dart';

/// A keyframe at [frame] of a 25 fps comp, eased both sides — what the F9
/// family stamps, and what a curve in the graph editor is made of.
///
/// Influence is a **fraction of the gap to the next key**, not a percentage:
/// the graph maths clamp it to 1, so a 33 here would silently become a
/// 100%-influence handle and every curve in the shots would be a staircase.
BridgeKeyframe _key(int frame, double value) => BridgeKeyframe(
      time: BridgeRational(num: frame, den: 25),
      value: value,
      interpIn: const BridgeSideInterp.bezier(
          BridgeBezierSide(speed: 0, influence: 0.33)),
      interpOut: const BridgeSideInterp.bezier(
          BridgeBezierSide(speed: 0, influence: 0.33)),
    );

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: const BridgeCompSettings(
      name: 'Opening titles',
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: 10, den: 1),
    ),
  );

  // Bottom of the stack upwards — each call puts its layer on top of the last —
  // so this reads as the running order of a title sequence: a music bed, a
  // background, the gameplay, the card over it, the logo, the words, and a
  // grade over the lot.
  final logoItem = project.importFootage(path: '$fixtures/Logo.png');
  for (final file in ['Music.wav']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  comp.addSolidLayer();
  for (final file in ['Gameplay.mp4', 'Title card.mp4']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  comp.addFootageLayer(footage: logoItem, asSequence: false);
  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(
      text: 'Northern lights',
      size: 140,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );
  comp.addAdjustmentLayer();

  final layers = comp.getLayers();
  for (final (index, name) in [
    'Grade',
    'Title',
    'Logo',
    'Title card',
    'Gameplay',
    'Background',
    'Music',
  ].indexed) {
    layers[index].rename(name: name);
  }
  final card = layers[3];
  final music = layers[6];

  // The title's own animation, spread across the ten seconds the way somebody
  // would actually key it: up and in over the first two, held, away again at
  // the end. Both axes of a pair, because a pair row draws its diamonds from
  // the axes it has and one static half shows none at all.
  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: [
    BridgeScalar.keyframed(
        [_key(0, 470), _key(50, 490), _key(200, 490), _key(240, 470)]),
    BridgeScalar.keyframed(
        [_key(0, 900), _key(50, 840), _key(200, 840), _key(240, 900)]),
  ]);
  title.setTransform(
    prop: BridgeTransformProp.opacity,
    value: BridgeScalar.keyframed(
        [_key(0, 0), _key(25, 100), _key(215, 100), _key(245, 0)]),
  );
  title.setTransforms(props: const [
    BridgeTransformProp.scaleX,
    BridgeTransformProp.scaleY,
  ], values: [
    BridgeScalar.keyframed([_key(0, 104), _key(50, 100), _key(240, 100)]),
    BridgeScalar.keyframed([_key(0, 104), _key(50, 100), _key(240, 100)]),
  ]);

  // A still arrives at a default length that reads as a sliver on the row;
  // placed, it runs for the four seconds it is on screen for.
  layers[2].setSpan(
    span: BridgeSpan(
      inPoint: comp.timeOfFrame(frame: 62),
      outPoint: comp.timeOfFrame(frame: 162),
      startOffset: comp.timeOfFrame(frame: 0),
    ),
  );

  // The title card sits over the footage at Screen, half up — a plausible
  // reason for the blend column to be showing anything but Normal.
  card.setTransform(
      prop: BridgeTransformProp.opacity, value: const BridgeScalar.static_(55));
  card.setBlend(index: 8);
  // Two switches away from their defaults, so the switches shot is a picture
  // of switches rather than of one repeated default.
  card.setSwitch(switch_: BridgeLayerSwitch.motionBlur, on_: true);
  // A music bed nobody wants to nudge by accident.
  music.setSwitch(switch_: BridgeLayerSwitch.locked, on_: true);

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;

  runApp(shotRoot(LumitAppNew(state, ui)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  final titleId = title.internallayerId.toString();
  final cardId = card.internallayerId.toString();
  final musicId = music.internallayerId.toString();

  /// Move the splitter between the upper band and the Timeline, and let the
  /// relayout settle. A deep fold needs the room, and dragging this is what
  /// anybody does when one will not fit.
  Future<void> timelineShare(double share) async {
    ui.workspace.dock.shares[0] = 1 - share;
    ui.workspace.dock.shares[1] = share;
    ui.workspace.touch();
    await pause(2);
  }

  /// The whole Timeline panel: the ruler gives the seam between outline and
  /// lanes and the panel's right edge, the zoom slider is its floor, and the
  /// outline runs to the window's left edge.
  Rect panelBox() {
    final ruler = boxOf('tl-ruler')!;
    return Rect.fromLTRB(
        2, ruler.top - 28, ruler.right + 4, boxOf('tl-zoom-slider')!.bottom + 8);
  }

  // ---- Shot: the Timeline, outline left and lanes right -------------------
  await captureUi('timeline.png', scale: 2, crop: panelBox());

  // ---- Shot: the outline columns, left to right ---------------------------
  // Cut at the seam: the caption is about the columns, so the lanes are not in
  // the picture at all.
  await captureUi(
    'timeline-outline.png',
    scale: 2,
    crop: Rect.fromLTRB(2, panelBox().top, boxOf('tl-ruler')!.left - 2,
        boxOf('tl-rowbody-$musicId')!.bottom + 10),
  );

  // ---- Shot: the switches on a layer's row --------------------------------
  // Both blocks of them — the eye/speaker/solo/lock/shy group and the
  // collapse/effects/motion-blur/3D group — with the column headers above and
  // the neighbouring rows either side, so it reads as a row rather than as a
  // strip of loose glyphs.
  final firstSwitch = boxOf('tl-visible-$cardId')!;
  await captureUi(
    'layer-switches.png',
    scale: 3,
    crop: Rect.fromLTRB(
      firstSwitch.left - 6,
      firstSwitch.top - 44,
      boxOf('tl-matte-$cardId')!.left - 6,
      firstSwitch.bottom + 22,
    ),
  );

  // ---- Shot: the blend-mode dropdown --------------------------------------
  // The list is taller than the room under the row, so it opens upwards over
  // the Viewer — which is where the crop has to reach to show it.
  await tapKey('tl-blend-$cardId', settle: 1.4);
  await captureUi(
    'blend-modes.png',
    scale: 2,
    crop: Rect.fromLTRB(2, 384, boxOf('tl-ruler')!.left + 40, 960),
  );
  // Close it again — the click on empty ground a person would use.
  await tapAt(const Offset(240, 560), settle: 1.2);

  // ---- Shots: a layer twirled open ----------------------------------------
  // Thirteen rows do not fit the arrangement's Timeline, so the splitter moves
  // first.
  await timelineShare(0.45);
  await tapKey('tl-twirl-$titleId');
  await tapKey('tl-twirl-${transformPath(titleId)}');
  await pause(1.5);
  await captureUi(
    'transform.png',
    scale: 2,
    crop: Rect.fromLTRB(
      2,
      boxOf('tl-rowbody-$titleId')!.top - 4,
      boxOf('tl-ruler')!.left + 330,
      boxOf('kf-stopwatch-tl-tf-opacity')!.bottom + 6,
    ),
  );
  await captureUi('timeline-lanes.png', scale: 2, crop: panelBox());

  // ---- Shot: keyframes on a property lane ---------------------------------
  // The lanes half only, over the open property rows, so what fills the
  // picture is diamonds on rows.
  final ruler = boxOf('tl-ruler')!;
  final position = boxOf('kf-stopwatch-tl-tf-positionX')!;
  await captureUi(
    'keyframes-lane.png',
    scale: 2,
    crop: Rect.fromLTRB(
        ruler.left, position.top - 70, ruler.right + 4, position.bottom + 92),
  );

  // ---- Shot: the stopwatch beside Position, with two keyframes ------------
  // This one belongs to the first-composition walkthrough, where exactly one
  // property has been animated and it has exactly two keys — so the layer is
  // put back to that state for the shot and given its full animation again
  // afterwards. Only Position's stopwatch is lit, which is what makes the
  // control legible: the same button, dark, sits beside every other row.
  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: [
    BridgeScalar.keyframed([_key(25, 470), _key(100, 490)]),
    BridgeScalar.keyframed([_key(25, 900), _key(100, 840)]),
  ]);
  title.setTransforms(props: const [
    BridgeTransformProp.scaleX,
    BridgeTransformProp.scaleY,
    BridgeTransformProp.opacity,
  ], values: const [
    BridgeScalar.static_(100),
    BridgeScalar.static_(100),
    BridgeScalar.static_(100),
  ]);
  ui.model.refresh();
  await pause(2);
  final layerRow = boxOf('tl-rowbody-$titleId')!;
  final opacityRow = boxOf('kf-stopwatch-tl-tf-opacity')!;
  await captureUi(
    'keyframes-stopwatch.png',
    scale: 2,
    crop: Rect.fromLTRB(
        2, layerRow.top - 4, ruler.left + 440, opacityRow.bottom + 2),
  );
  await tapKey('tl-twirl-$titleId');

  // ---- Shot: a footage layer's waveform in its lane -----------------------
  await tapKey('tl-twirl-$musicId');
  await tapKey('tl-twirl-${audioPath(musicId)}');
  await tapKey('tl-twirl-${waveformPath(musicId)}');
  // The peaks are fetched from the engine and arrive a moment later.
  await pause(5);
  final wave = boxOf('tl-wave-$musicId')!;
  await captureUi(
    'waveform.png',
    scale: 2,
    crop: Rect.fromLTRB(120, boxOf('tl-rowbody-$musicId')!.top - 2,
        boxOf('tl-ruler')!.right + 4, wave.bottom + 8),
  );
  await tapKey('tl-twirl-$musicId');
  await timelineShare(0.32);

  // ---- Shot: the cache bar filling ----------------------------------------
  // Filled by the engine's own idle fill, which banks frames forward from the
  // playhead whenever nothing else is asking for the card. A minute of sitting
  // still is what a bar that is *filling* looks like — and this is before the
  // beats below, so nothing is drawn over it.
  ui.playheadFrame.value = 0;
  // Emptied first: left alone long enough the engine banks the whole
  // composition, and a full bar is not the one the page is about.
  clearCache();
  clearVramCache();
  ui.cacheChanged.value++;
  // Only a few seconds of it: the engine banks this composition in well under
  // a minute, and a bar that has finished is not a bar filling.
  await pause(4);
  final held = comp.cachedFrames(frames: BigInt.from(250), scale: ui.viewerScale);
  // ignore: avoid_print
  print('CACHE ${held.where((t) => t != 0).length} of ${held.length} '
      'at scale ${ui.viewerScale}');
  final bar = boxOf('tl-cache-bar')!;
  // Four image pixels per logical one: the bar is two logical pixels tall, and
  // a hairline is not a screenshot of a control.
  await captureUi(
    'cache-bar.png',
    scale: 4,
    crop: Rect.fromLTRB(boxOf('tl-ruler')!.left - 2, bar.top - 40,
        boxOf('tl-ruler')!.right + 4, bar.bottom),
  );

  // ---- Shot: comp markers and beat markers on the ruler -------------------
  // The cues somebody would leave themselves, and then the beats the engine
  // really finds in the music underneath them. One-word cues: a flag is as
  // wide as its label, and a sentence-long one covers the beats either side
  // of it — which is the one thing this shot has to show apart.
  comp.setMarkers(markers: [
    for (final (at, label) in [(62, 'Logo'), (138, 'Cut')])
      BridgeMarker(
        id: UuidValue.fromString(const Uuid().v4()),
        time: BridgeRational(num: at, den: 25),
        label: label,
      ),
  ]);
  // Detection at the sensitivity that marks the beat rather than every
  // transient in the bar: the fixture's hits come four to the second, and a
  // ruler carrying forty flags cannot show a comp marker as a different thing.
  var beats = 0;
  for (final sensitivity in [50, 30, 18, 10, 5]) {
    beats = await comp.detectBeats(sensitivityPercent: sensitivity);
    // ignore: avoid_print
    print('BEATS $beats at $sensitivity');
    if (beats <= 14) break;
  }
  ui.model.refresh();
  ui.playheadFrame.value = 48;
  await pause(2);
  final ruler2 = boxOf('tl-ruler')!;
  await captureUi(
    'markers-ruler.png',
    scale: 2,
    crop: Rect.fromLTRB(
        ruler2.left - 2, ruler2.top - 4, ruler2.right + 4, ruler2.bottom + 4),
  );

  exit(0);
}
