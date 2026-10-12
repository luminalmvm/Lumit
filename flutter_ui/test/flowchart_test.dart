// The flowchart popover: what it draws round a composition, where the arrow
// keys take the cursor, and what opens and shuts it.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/flowchart_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:uuid/uuid.dart';

CompositionReference _comp(int n) => CompositionReference(
      internalproject:
          UuidValue.fromString('00000000-0000-4000-8000-000000000000'),
      internalid: UuidValue.fromString(
          '00000000-0000-4000-8000-${n.toString().padLeft(12, '0')}'),
    );

void main() {
  // Film and Reel place Shot, Shot places Plate and Titles, Plate places
  // Grain. Solo is in nothing and holds nothing.
  final film = _comp(1), reel = _comp(2), shot = _comp(3);
  final plate = _comp(4), titles = _comp(5), grain = _comp(6), solo = _comp(7);
  BridgeCompFlowLink link(CompositionReference comp, String name,
          {bool more = false}) =>
      BridgeCompFlowLink(comp: comp, name: name, more: more);
  final flows = <CompositionReference, BridgeCompFlow>{
    film: BridgeCompFlow(
        name: 'Film', usedBy: const [], uses: [link(shot, 'Shot', more: true)]),
    shot: BridgeCompFlow(name: 'Shot', usedBy: [
      link(film, 'Film'),
      link(reel, 'Reel'),
    ], uses: [
      link(plate, 'Plate', more: true),
      link(titles, 'Titles'),
    ]),
    plate: BridgeCompFlow(
        name: 'Plate',
        usedBy: [link(shot, 'Shot', more: true)],
        uses: [link(grain, 'Grain')]),
    solo: const BridgeCompFlow(name: 'Solo', usedBy: [], uses: []),
  };

  final opened = <CompositionReference>[];
  setUp(opened.clear);

  // Held across a test, since a second chart in one test opens over the
  // overlay the first one mounted.
  late BuildContext ctx;
  Future<bool> open(WidgetTester tester, CompositionReference comp,
      {Offset? anchor}) async {
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(initialEntries: [
          OverlayEntry(builder: (context) {
            ctx = context;
            return const SizedBox.expand();
          }),
        ]),
      ),
    ));
    final shown = showFlowchartFrb(
      context: ctx,
      comp: comp,
      anchor: anchor,
      onOpen: opened.add,
      read: (c) => flows[c],
    );
    await tester.pump();
    await tester.pump();
    return shown;
  }

  Future<void> press(WidgetTester tester, List<LogicalKeyboardKey> keys) async {
    for (final key in keys) {
      await tester.sendKeyEvent(key);
      await tester.pump();
    }
  }

  final chart = find.byKey(const ValueKey('flowchart'));
  double x(WidgetTester tester, String name) =>
      tester.getCenter(find.text(name)).dx;

  testWidgets('a comp sits between what uses it and what it contains',
      (tester) async {
    expect(await open(tester, shot), isTrue);
    final t = LumitTheme.dark();
    expect(find.text(t.kickerCase('Used in')), findsOneWidget);
    expect(find.text(t.kickerCase('Contains')), findsOneWidget);
    for (final user in ['Film', 'Reel']) {
      expect(x(tester, user), lessThan(x(tester, 'Shot')));
    }
    for (final nested in ['Plate', 'Titles']) {
      expect(x(tester, nested), greaterThan(x(tester, 'Shot')));
    }
    expect(find.text('Grain'), findsNothing, reason: 'one step each way');
    expect(lumitModalOpen, isTrue, reason: 'the keyboard is the chart\'s');
  });

  testWidgets('Enter opens the comp under the cursor', (tester) async {
    await open(tester, shot);
    await press(tester, [
      LogicalKeyboardKey.arrowRight,
      LogicalKeyboardKey.arrowDown,
      LogicalKeyboardKey.enter,
    ]);
    expect(opened, [titles]);
    expect(chart, findsNothing);
    expect(lumitModalOpen, isFalse);
  });

  testWidgets('the cursor comes back through the middle', (tester) async {
    await open(tester, shot);
    await press(tester, [
      LogicalKeyboardKey.arrowRight,
      LogicalKeyboardKey.arrowLeft,
      LogicalKeyboardKey.arrowLeft,
      LogicalKeyboardKey.arrowDown,
      LogicalKeyboardKey.enter,
    ]);
    expect(opened, [reel]);
  });

  testWidgets('Enter on the comp already open only shuts the chart',
      (tester) async {
    await open(tester, shot);
    await press(tester, [LogicalKeyboardKey.enter]);
    expect(opened, isEmpty);
    expect(chart, findsNothing);
  });

  testWidgets('arrowing past a comp that carries on slides the chart to it',
      (tester) async {
    await open(tester, shot);
    await press(
        tester, [LogicalKeyboardKey.arrowRight, LogicalKeyboardKey.arrowRight]);
    expect(find.text('Grain'), findsOneWidget, reason: 'Plate is the middle');
    expect(find.text('Film'), findsNothing);
    expect(x(tester, 'Shot'), lessThan(x(tester, 'Plate')));
    await press(tester, [LogicalKeyboardKey.enter]);
    expect(opened, [plate], reason: 'the cursor stayed on Plate');
  });

  testWidgets('a comp with nothing past it stays put', (tester) async {
    await open(tester, shot);
    await press(tester, [
      LogicalKeyboardKey.arrowRight,
      LogicalKeyboardKey.arrowDown,
      LogicalKeyboardKey.arrowRight,
    ]);
    expect(find.text('Film'), findsOneWidget, reason: 'Shot is the middle');
    await press(tester, [LogicalKeyboardKey.enter]);
    expect(opened, [titles]);
  });

  testWidgets('the stub slides the chart without opening anything',
      (tester) async {
    await open(tester, shot);
    await tester.tap(find.byKey(const ValueKey('flowchart-more-uses-0')));
    await tester.pump();
    expect(find.text('Grain'), findsOneWidget);
    expect(opened, isEmpty);
    expect(chart, findsOneWidget);
  });

  testWidgets('a click opens a comp, and a click outside shuts the chart',
      (tester) async {
    await open(tester, shot);
    await tester.tap(find.text('Film'));
    await tester.pump();
    expect(opened, [film]);
    expect(chart, findsNothing);

    await open(tester, shot);
    await tester.tapAt(const Offset(5, 5));
    await tester.pump();
    expect(opened, [film], reason: 'nothing more was opened');
    expect(chart, findsNothing);
  });

  testWidgets('Tab shuts it, and so does Escape', (tester) async {
    for (final key in [LogicalKeyboardKey.tab, LogicalKeyboardKey.escape]) {
      await open(tester, shot);
      expect(chart, findsOneWidget);
      await press(tester, [key]);
      expect(chart, findsNothing, reason: key.keyLabel);
    }
    expect(opened, isEmpty);
  });

  testWidgets('the middle pill opens on the anchor', (tester) async {
    await open(tester, shot, anchor: const Offset(400, 300));
    final middle =
        tester.getCenter(find.byKey(const ValueKey('flowchart-centre')));
    expect(middle, const Offset(400, 300));
  });

  testWidgets('an anchor off the edge is pulled in so the chart fits',
      (tester) async {
    await open(tester, shot, anchor: const Offset(2, 2));
    final at = tester.getTopLeft(chart);
    expect(at.dx, greaterThanOrEqualTo(8));
    expect(at.dy, greaterThanOrEqualTo(8));
  });

  testWidgets('a comp on its own says so', (tester) async {
    await open(tester, solo);
    expect(find.text('Solo'), findsOneWidget);
    final t = LumitTheme.dark();
    expect(find.text(t.kickerCase('Used in')), findsNothing);
    expect(
        find.text('Not nested, and nothing is nested in it'), findsOneWidget);
  });

  testWidgets('a comp that has gone opens no chart', (tester) async {
    expect(await open(tester, grain), isFalse);
    expect(chart, findsNothing);
    expect(lumitModalOpen, isFalse);
  });
}
