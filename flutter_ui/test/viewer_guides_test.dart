
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_rulers.dart';
import 'package:lumit_flutter/panels/viewer_snap.dart';
import 'package:lumit_flutter/state/workspace.dart';

void main() {
  const picture = Rect.fromLTWH(20, 20, 384, 216);
  const compSize = Size(1920, 1080);

  group('The magnet on the picture', () {
    /// **Each axis decides on its own**: a layer held against a guide down one
    /// side still slides freely along it, which is what a guide is for.
    test('nudges a dragged box onto a guide, one axis at a time', () {
      const box = Rect.fromLTWH(50, 300, 100, 50);
      final nudged = snapViewerDrag(
        box: box,
        delta: const Offset(147, 40),
        verticals: const [200],
        horizontals: const [],
      );
      expect(nudged, (delta: const Offset(150, 40), x: 200.0, y: null));

      expect(
        snapViewerDrag(
            box: box,
            delta: const Offset(120, 40),
            verticals: const [200],
            horizontals: const []),
        (delta: const Offset(120, 40), x: null, y: null),
      );

      // The middle of the box counts too, not only its edges: it is what
      // anybody centring a layer on a guide is aiming with.
      expect(
        snapViewerDrag(
                box: box,
                delta: const Offset(97, 0),
                verticals: const [200],
                horizontals: const [])
            .delta
            .dx,
        100,
      );
    });
  });

  group('Guides come out of the rulers', () {
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
      expect(guides.single.at, closeTo(400, 1e-6));
    });

    testWidgets('an existing guide moves, and drops back into the ruler to go',
        (tester) async {
      final held = await mount(tester, guides: const [(at: 500, vertical: false)]);

      await tester.dragFrom(const Offset(200, 120), const Offset(0, 40));
      await tester.pumpAndSettle();
      expect(held().single.at, closeTo(700, 1e-6));

      await tester.dragFrom(const Offset(200, 160), const Offset(0, -155));
      await tester.pumpAndSettle();
      expect(held(), isEmpty);
    });
  });

  /// **The overlays and the guides ride the session**: they are written with
  /// the rest of where the user was, and read back with it. A session from a
  /// build that had neither reads as a comp with nothing drawn on it rather
  /// than failing to open.
  test('the session carries the overlays and the guides', () {
    const session = SavedSession(
      activeComp: 'a',
      viewerOverlays: {'a': (grid: true, safeAreas: false, rulers: true, reach: false)},
      guides: {
        'a': [(at: 960, vertical: true), (at: 540, vertical: false)],
      },
    );
    final back =
        SavedSession.fromJson(jsonDecode(jsonEncode(session.toJson())) as Map<String, dynamic>);
    expect(back.viewerOverlays['a'],
        (grid: true, safeAreas: false, rulers: true, reach: false));
    expect(back.guides['a'], session.guides['a']);
    // Equal sessions must compare equal, or the session file would be
    // rewritten on every frame (the reason the regions are keyed as text).
    expect(back, session);
    expect(back.hashCode, session.hashCode);

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
