// The long-list searchable picker (K-262, retargeted K-264).
//
// Built for the Lens flare's 1299-lens library, whose eager dropdown took the
// app down in a layout pass. The library is twenty curated lenses now (K-264)
// — below the searchable threshold, so the flare uses the plain dropdown —
// but the picker remains the guard for ANY long Choice list, so its laziness
// and its search are pinned here against synthetic options rather than
// against an effect that no longer exercises them.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  Widget host(Widget child) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(builder: (_) => Center(child: child)),
            ],
          ),
        ),
      );

  // A library-shaped option list: many makers, many models.
  final options = [
    for (var maker = 0; maker < 40; maker++)
      for (var model = 0; model < 10; model++)
        'Maker$maker · Model $model${maker == 7 ? ' special' : ''}',
  ];

  testWidgets('the searchable picker builds lazily and narrows by search',
      (tester) async {
    var picked = -1;
    await tester.pumpWidget(host(SizedBox(
      width: 200,
      child: BareSearchDropdown(
        value: 3,
        options: options,
        group: (label) {
          final i = label.indexOf(' · ');
          return i > 0 ? label.substring(0, i) : null;
        },
        onChanged: (i) => picked = i,
      ),
    )));

    await tester.tap(find.byType(BareSearchDropdown));
    await tester.pumpAndSettle();

    // A search field opened, and only a screenful of rows exists — the
    // whole point: 400 eager rows is the crash this widget prevents.
    expect(find.byType(HouseTextField), findsOneWidget);
    final rows = tester.widgetList(find.byType(MenuRow)).length;
    expect(rows, lessThan(80), reason: 'the picker must build lazily');

    // Typing narrows to the matching maker's models.
    await tester.enterText(find.byType(HouseTextField), 'maker7 special');
    await tester.pumpAndSettle();
    final narrowed = tester.widgetList(find.byType(MenuRow)).length;
    expect(narrowed, lessThan(rows));
    expect(narrowed, greaterThan(0));

    // A query matching nothing says so rather than showing a blank sheet.
    await tester.enterText(find.byType(HouseTextField), 'qqqzzz');
    await tester.pumpAndSettle();
    expect(find.text('No matches'), findsOneWidget);

    // Back to one maker: picking a row reports its ORIGINAL index.
    await tester.enterText(find.byType(HouseTextField), 'maker7 · model 4');
    await tester.pumpAndSettle();
    await tester.tap(find.byType(MenuRow).first);
    await tester.pumpAndSettle();
    expect(picked, 74, reason: 'index into the full option list');
  });
}
