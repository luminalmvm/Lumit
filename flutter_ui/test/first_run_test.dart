// The first-run screen: asked once, on a machine with no settings
// file, and its editing answer sets the two editing preferences.
//
// The screen is worth its own tests because everything about it is a
// once-only side effect: an answer that did not stick would send the user
// round again, and a screen that appeared on the second launch would be worse
// than not having one.

import 'dart:io';

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/first_run_frb.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/theme_swatches.dart';

void main() {
  late Workspace workspace;

  setUp(() {
    // Never the developer's own settings file.
    Workspace.storeOverride =
        '${Directory.systemTemp.path}/lumit-first-run-test.json';
    workspace = Workspace()..firstRunDone = false;
  });

  tearDown(() => Workspace.storeOverride = null);

  /// An app that puts the screen up as soon as it has an Overlay, the way the
  /// shell does — from `initState`, **once**. Asking from the overlay entry's
  /// builder instead would re-ask on every rebuild, and since inserting the
  /// screen is itself a rebuild, that stacks screens for ever. The shell is
  /// safe from this because `_LumitAppViewState.initState` runs once; a test
  /// host that differed there would be testing a shell nobody ships.
  Widget host() => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(builder: (_) => _AskOnce(workspace: workspace)),
            ],
          ),
        ),
      );

  testWidgets('the Vegas answer sets both preferences', (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('first-run-vegas')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('first-run-vegas')));
    await tester.pumpAndSettle();
    // A choice is only a choice until Continue keeps it.
    expect(workspace.firstRunDone, isFalse);
    await tester.tap(find.byKey(const ValueKey('first-run-continue')));
    await tester.pumpAndSettle();

    expect(workspace.interface.retimeOpensToSpeed, isTrue);
    expect(workspace.interface.videoAsSequenceLayer, isTrue);
    expect(workspace.firstRunDone, isTrue);
    expect(find.byKey(const ValueKey('first-run-vegas')), findsNothing,
        reason: 'answering closes the screen');
  });

  testWidgets('unticking the update box is remembered', (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('first-run-auto-update')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('first-run-vegas')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('first-run-continue')));
    await tester.pumpAndSettle();

    expect(workspace.autoUpdate, isFalse);
    // The editing answer is unaffected: two questions, one screen.
    expect(workspace.interface.videoAsSequenceLayer, isTrue);
  });

  /// Hovering a row of the open list draws the interface in that scheme and
  /// saves nothing, so leaving the list or clicking away puts the old one back.
  testWidgets('hovering a scheme shows it and does not choose it',
      (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();
    final chosen = workspace.themeChoice;
    final own = swatchesOf(workspace.theme);
    final other = workspace.themeChoices.firstWhere((c) => c != chosen);

    await tester.tap(find.byKey(const ValueKey('first-run-scheme')));
    await tester.pumpAndSettle();
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.text(other.label)));
    await tester.pumpAndSettle();

    expect(swatchesOf(workspace.theme), isNot(own),
        reason: 'the scheme under the pointer is the one drawn');
    expect(workspace.themeChoice, chosen, reason: 'and nothing was chosen');

    // Off the list, with it still open.
    await mouse.moveTo(const Offset(2, 2));
    await tester.pumpAndSettle();
    expect(swatchesOf(workspace.theme), own);

    // Back on the row, then a click away: dismissed while looking.
    await mouse.moveTo(tester.getCenter(find.text(other.label)));
    await tester.pumpAndSettle();
    expect(swatchesOf(workspace.theme), isNot(own));
    await tester.tapAt(const Offset(2, 2));
    await tester.pumpAndSettle();
    expect(swatchesOf(workspace.theme), own);
    expect(workspace.themeChoice, chosen);

    // And picking a row still picks it.
    await tester.tap(find.byKey(const ValueKey('first-run-scheme')));
    await tester.pumpAndSettle();
    await tester.tap(find.text(other.label));
    await tester.pumpAndSettle();
    expect(workspace.themeChoice, other);
    expect(swatchesOf(workspace.theme), isNot(own));
  });

  testWidgets('a machine that has answered is never asked again',
      (tester) async {
    workspace.firstRunDone = true;
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('first-run-vegas')), findsNothing);
  });
}

/// The shell's own arrangement: ask after the first frame, from `initState`,
/// so the question is put once however often the tree rebuilds.
class _AskOnce extends StatefulWidget {
  final Workspace workspace;
  const _AskOnce({required this.workspace});

  @override
  State<_AskOnce> createState() => _AskOnceState();
}

class _AskOnceState extends State<_AskOnce> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) maybeShowFirstRunFrb(context, widget.workspace);
    });
  }

  @override
  Widget build(BuildContext context) => const SizedBox.expand();
}
