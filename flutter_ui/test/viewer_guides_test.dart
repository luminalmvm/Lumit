// Rulers, guides and snapping on the picture (docs/07 §2.2 item 6).
//
// Three things are worth checking and each is checked where it lives: the
// ruler's arithmetic and the magnet's are pure, so they are computed by hand
// here; making, moving and deleting a guide is a gesture, so it is dragged in a
// widget tree; and a guide surviving the day is a question about the session's
// JSON, so it is written and read back.

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_rulers.dart';
import 'package:lumit_flutter/panels/viewer_snap.dart';
import 'package:lumit_flutter/state/workspace.dart';

void main() {
  // The picture, as the stage would have it: an HD comp drawn at a fifth of
  // its size, 20 in from the stage's corner.
  const picture = Rect.fromLTWH(20, 20, 384, 216);
  const compSize = Size(1920, 1080);

  group('The ruler counts comp pixels', () {
    /// **The step is a 1 / 2 / 5 rung, chosen by what it looks like on screen**
    /// — which is what keeps the numbers round at every magnification and the
    /// labels from colliding when the picture is small.
    test('steps up the ladder as the picture shrinks', () {
      // At 1:1, 64 screen pixels want 64 comp pixels: the first rung at or
      // past it is 100.
      expect(viewerRulerStep(1), 100);
      // Zoomed to 8×, 64 screen pixels are 8 comp pixels — the rung is 10.
      expect(viewerRulerStep(8), 10);
      // A fifth of size: 64 screen pixels are 320 comp pixels, so 500.
      expect(viewerRulerStep(0.2), 500);
      // Every answer keeps labels at least the asked-for gap apart.
      for (final scale in [0.02, 0.2, 1.0, 3.0, 32.0]) {
        expect(viewerRulerStep(scale) * scale,
            greaterThanOrEqualTo(viewerRulerLabelGap),
            reason: 'labels must not collide at ${scale}x');
      }
      // A collapsed picture answers rather than dividing by zero.
      expect(viewerRulerStep(0), 100);
      expect(viewerRulerStep(double.nan), 100);
    });

    /// A guide is kept in comp pixels and drawn from the picture's rectangle,
    /// which is the whole of why guides pan and zoom with the shot.
    test('turns comp pixels into places on screen, and back', () {
      const guide = (at: 960.0, vertical: true);
      final at =
          viewerGuideScreen(guide, picture: picture, compSize: compSize);
      expect(at, 20 + 960 * 0.2);
      expect(
        viewerGuideComp(at,
            vertical: true, picture: picture, compSize: compSize),
        closeTo(960, 1e-9),
      );
      // The other axis has its own scale, and reads from the picture's top.
      expect(
        viewerGuideScreen((at: 540.0, vertical: false),
            picture: picture, compSize: compSize),
        20 + 540 * 0.2,
      );
    });
  });

  group('The magnet on the picture', () {
    /// **It engages inside the slop and not outside it**, measured in screen
    /// pixels — the Timeline's own rule, which is what makes the magnification
    /// the precision control.
    test('takes the nearest line within reach, and nothing beyond it', () {
      // Seven pixels short: caught, and pulled the whole seven.
      expect(snapToLines(moving: const [93], targets: const [100]),
          (shift: 7.0, caught: 100.0));
      // Nine short: out of reach, and nothing moves.
      expect(snapToLines(moving: const [91], targets: const [100]),
          noViewerSnap);
      // Nearest wins, whichever side it is on.
      expect(snapToLines(moving: const [99], targets: const [96, 100]),
          (shift: 1.0, caught: 100.0));
      // An edge already on a line is pinned there, which is a shift of zero
      // rather than no catch at all.
      expect(snapToLines(moving: const [100], targets: const [100]),
          (shift: 0.0, caught: 100.0));
      expect(snapToLines(moving: const [], targets: const [100]), noViewerSnap);
      expect(snapToLines(moving: const [100], targets: const []), noViewerSnap);
    });

    /// The list a drag reaches for: the guides that run the right way, and the
    /// grid only when the menu says so.
    test('gathers the guides of one axis, and the grid only when asked', () {
      const guides = <ViewerGuide>[
        (at: 960, vertical: true),
        (at: 540, vertical: false),
      ];
      expect(
        viewerSnapTargets(
            guides: guides,
            vertical: true,
            picture: picture,
            compSize: compSize),
        [20 + 960 * 0.2],
      );
      final withGrid = viewerSnapTargets(
        guides: guides,
        vertical: false,
        picture: picture,
        compSize: compSize,
        grid: true,
      );
      // The horizontal guide, then the nine lines of the frame's eighths —
      // the two edges included, because an edge is a thing to line up with.
      expect(withGrid.first, 20 + 540 * 0.2);
      expect(withGrid.length, 1 + 9);
      expect(withGrid.contains(picture.top), isTrue);
      expect(withGrid.contains(picture.bottom), isTrue);
    });

    /// **Each axis decides on its own**: a layer held against a guide down one
    /// side still slides freely along it, which is what a guide is for.
    test('nudges a dragged box onto a guide, one axis at a time', () {
      // A 100×50 box whose left edge would land three pixels short of a
      // vertical line at 200, and whose vertical travel reaches nothing.
      const box = Rect.fromLTWH(50, 300, 100, 50);
      final nudged = snapViewerDrag(
        box: box,
        delta: const Offset(147, 40),
        verticals: const [200],
        horizontals: const [],
      );
      expect(nudged, const Offset(150, 40));

      // Out of reach on both axes: the pointer's own travel, untouched.
      expect(
        snapViewerDrag(
            box: box,
            delta: const Offset(120, 40),
            verticals: const [200],
            horizontals: const []),
        const Offset(120, 40),
      );

      // The middle of the box counts too, not only its edges: it is what
      // anybody centring a layer on a guide is aiming with.
      expect(
        snapViewerDrag(
                box: box,
                delta: const Offset(97, 0),
                verticals: const [200],
                horizontals: const [])
            .dx,
        100,
      );
    });
  });

  group('Guides come out of the rulers', () {
    /// Mount the layer on its own — it draws in the stage's coordinates and
    /// needs nothing else — and hand back what it last wrote.
    Future<List<ViewerGuide> Function()> mount(
      WidgetTester tester, {
      List<ViewerGuide> guides = const [],
      bool rulers = true,
    }) async {
      var held = guides;
      tester.view.physicalSize = const Size(500, 400);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 500,
            height: 400,
            child: StatefulBuilder(
              builder: (context, setState) => Stack(
                children: [
                  ViewerRulers(
                    rulers: rulers,
                    picture: picture,
                    compSize: compSize,
                    guides: held,
                    onGuides: (next) => setState(() => held = next),
                    band: const Color(0xFF202020),
                    line: const Color(0xFF404040),
                    label: const Color(0xFF808080),
                    guideColour: const Color(0xFF00A0A0),
                  ),
                ],
              ),
            ),
          ),
        ),
      ));
      return () => held;
    }

    /// **A guide is dragged out of a ruler and lands in comp pixels** — the
    /// top strip makes a horizontal one, which is the way round every editor
    /// has taught.
    testWidgets('a drag out of the top strip makes a horizontal guide',
        (tester) async {
      final held = await mount(tester);

      await tester.dragFrom(
          const Offset(200, viewerRulerBand / 2), const Offset(0, 91));
      await tester.pumpAndSettle();

      final guides = held();
      expect(guides.length, 1);
      expect(guides.single.vertical, isFalse);
      // Dropped at y = 9 + 91 = 100, which is 80 screen pixels down the
      // picture, which is 400 comp pixels at a fifth of size.
      expect(guides.single.at, closeTo(400, 1e-6));
    });

    /// The left strip makes the other kind, and a guide dropped **off the
    /// picture** is not made at all — which is what makes a drag started by
    /// accident cost nothing.
    testWidgets('a drag that ends off the picture makes nothing',
        (tester) async {
      final held = await mount(tester);

      await tester.dragFrom(
          const Offset(viewerRulerBand / 2, 200), const Offset(450, 0));
      await tester.pumpAndSettle();
      expect(held(), isEmpty);

      await tester.dragFrom(
          const Offset(viewerRulerBand / 2, 200), const Offset(200, 0));
      await tester.pumpAndSettle();
      final guides = held();
      expect(guides.length, 1);
      expect(guides.single.vertical, isTrue);
      expect(guides.single.at, closeTo((209 - 20) / 0.2, 1e-6));
    });

    /// **A guide moves by its own grab strip, and dragging it back onto a
    /// ruler deletes it** — the whole of a guide's lifecycle, in two drags.
    testWidgets('an existing guide moves, and drops back into the ruler to go',
        (tester) async {
      final held = await mount(tester, guides: const [(at: 500, vertical: false)]);

      // It draws at 20 + 500 × 0.2 = 120 down the stage.
      await tester.dragFrom(const Offset(200, 120), const Offset(0, 40));
      await tester.pumpAndSettle();
      expect(held().single.at, closeTo(700, 1e-6));

      // And back into the strip, which is how a guide is thrown away.
      await tester.dragFrom(const Offset(200, 160), const Offset(0, -155));
      await tester.pumpAndSettle();
      expect(held(), isEmpty);
    });

    /// The strips are the surface guides come out of, so with the rulers down
    /// there is nothing to pull one from — but the guides already placed are
    /// still drawn, because a line you put there is a thing you put there.
    testWidgets('with the rulers down there is no strip to drag from',
        (tester) async {
      final held = await mount(tester,
          rulers: false, guides: const [(at: 500, vertical: false)]);

      expect(find.byKey(const ValueKey('viewer-ruler-top')), findsNothing);
      expect(find.byKey(const ValueKey('viewer-guide-0')), findsOneWidget);
      await tester.dragFrom(
          const Offset(200, viewerRulerBand / 2), const Offset(0, 91));
      await tester.pumpAndSettle();
      expect(held().length, 1, reason: 'no new guide was made');
    });
  });

  /// **The overlays and the guides ride the session**: they are written with
  /// the rest of where the user was, and read back with it. A session from a
  /// build that had neither reads as a comp with nothing drawn on it rather
  /// than failing to open.
  test('the session carries the overlays and the guides', () {
    const session = SavedSession(
      activeComp: 'a',
      viewerOverlays: {'a': (grid: true, safeAreas: false, rulers: true)},
      guides: {
        'a': [(at: 960, vertical: true), (at: 540, vertical: false)],
      },
    );
    final back =
        SavedSession.fromJson(jsonDecode(jsonEncode(session.toJson())) as Map<String, dynamic>);
    expect(back.viewerOverlays['a'],
        (grid: true, safeAreas: false, rulers: true));
    expect(back.guides['a'], session.guides['a']);
    // Equal sessions must compare equal, or the session file would be
    // rewritten on every frame (the reason the regions are keyed as text).
    expect(back, session);
    expect(back.hashCode, session.hashCode);

    // Nonsense is dropped rather than half-read.
    final ragged = SavedSession.fromJson({
      'viewer_overlays': {'a': 'yes', 'b': <String, dynamic>{}},
      'guides': {
        'a': [
          {'at': 'x'},
          {'at': double.infinity},
          {'at': 12}
        ],
      },
    });
    expect(ragged.viewerOverlays, isEmpty,
        reason: 'a comp with nothing drawn is simply absent');
    expect(ragged.guides['a'], [(at: 12.0, vertical: false)]);
  });
}
