// The Settings window's scrollbar and the movable window under it (6.16), and
// the last static hint (6.12).
//
// **The bug this pins.** The Settings window is a movable window: dragging it
// anywhere no control claims moves it. Its page is a list with a 6px
// scrollbar, and with a mouse almost nothing on that scrollbar claims a drag —
// a list is not drag-scrolled by a mouse, and `RawScrollbar` answers only a
// tap on its track and a drag on its thumb. So reaching for the gutter and
// pulling did not scroll the page: it picked the whole dialog up and carried
// it across the screen.

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Settings scrolling (frb)', () {
    Future<void> open(WidgetTester tester,
        {String name = 'appearance', ScrollBehavior? behaviour}) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      final panel = hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(context),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1200, 900),
      );
      await tester.pumpWidget(behaviour == null
          ? panel
          : ScrollConfiguration(behavior: behaviour, child: panel));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();
      // Appearance is the long page — the one that has somewhere to scroll to.
      await tester.tap(find.byKey(ValueKey('settings-page-$name')));
      await tester.pumpAndSettle();
    }

    ScrollPosition page(WidgetTester tester, [String name = 'appearance']) =>
        tester
            .state<ScrollableState>(find.descendant(
              of: find.byKey(ValueKey('settings-body-$name')),
              matching: find.byType(Scrollable),
            ))
            .position;

    testWidgets('dragging the gutter scrolls the page and leaves the window',
        (tester) async {
      await open(tester);
      final before = tester.getRect(
          find.byKey(const ValueKey('settings-title-strip')));
      final gutter = tester.getRect(find.byKey(const ValueKey('settings-gutter')));
      expect(page(tester).maxScrollExtent, greaterThan(0),
          reason: 'the Appearance page must overflow for this to mean anything');

      // A mouse, because that is what the owner has in their hand and what
      // nothing else on the scrollbar answers.
      final drag = await tester.startGesture(
        Offset(gutter.center.dx, gutter.bottom - 20),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pump(const Duration(milliseconds: 40));
      final started = page(tester).pixels;
      for (var i = 0; i < 5; i++) {
        await drag.moveBy(const Offset(0, 8));
        await tester.pump(const Duration(milliseconds: 16));
      }
      await drag.up();
      await tester.pumpAndSettle();

      expect(page(tester).pixels, greaterThan(started),
          reason: 'pulling the gutter down scrolls the page down');
      expect(
        tester.getRect(find.byKey(const ValueKey('settings-title-strip'))),
        before,
        reason: 'the dialog stayed exactly where it was',
      );
    });

    // A lazy list only guesses the height of the rows it has not built, so
    // the guess, and the thumb drawn from it, moved as the page scrolled.
    for (final name in ['appearance', 'shortcuts']) {
      testWidgets('the $name page keeps one length while it scrolls',
          (tester) async {
        await open(tester, name: name);
        final length = page(tester, name).maxScrollExtent;
        expect(length, greaterThan(0));
        for (var at = 0.0; at <= length; at += 200) {
          page(tester, name).jumpTo(at);
          await tester.pump();
          expect(page(tester, name).maxScrollExtent, length,
              reason: 'the thumb is sized from this, so it must not move');
        }
      });
    }

    // The app's scroll behaviour adds a scrollbar of its own on the desktop,
    // which drew a second thumb beside the gutter's while the page moved.
    testWidgets('the page draws one thumb on the desktop', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      try {
        await open(tester, behaviour: const MaterialScrollBehavior());
        page(tester).jumpTo(100);
        await tester.pump();
        expect(
            find.byWidgetPredicate((w) => w is RawScrollbar), findsOneWidget);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    testWidgets('a tap on the track still pages, as it always did',
        (tester) async {
      await open(tester);
      final gutter = tester.getRect(find.byKey(const ValueKey('settings-gutter')));
      await tester.tapAt(Offset(gutter.center.dx, gutter.bottom - 20),
          kind: PointerDeviceKind.mouse);
      await tester.pumpAndSettle();
      expect(page(tester).pixels, greaterThan(0));
    });

    /// 6.12: The redesign leaves a row's second line to a *live report* — what
    /// the machine has, what a choice costs here and now. The Chrome labels
    /// row was the last one carrying a paragraph explaining itself.
    testWidgets('no static hint is left under a settings row', (tester) async {
      await open(tester);
      expect(
        find.textContaining('Hover text always says the word'),
        findsNothing,
        reason: 'the Chrome labels row explains itself by its own options',
      );
    });
  });
}
