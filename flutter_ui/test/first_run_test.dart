// The first-run screen (K-246): asked once, on a machine with no settings
// file, and its answer sets the two editing preferences.
//
// The screen is worth its own tests because everything about it is a
// once-only side effect: an answer that did not stick would send the user
// round again, and a screen that appeared on the second launch would be worse
// than not having one.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/first_run_frb.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

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

    expect(workspace.interface.retimeOpensToSpeed, isTrue);
    expect(workspace.interface.videoAsSequenceLayer, isTrue);
    expect(workspace.firstRunDone, isTrue);
    expect(find.byKey(const ValueKey('first-run-vegas')), findsNothing,
        reason: 'answering closes the screen');
  });

  testWidgets('the After Effects answer leaves both preferences off',
      (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('first-run-ae')));
    await tester.pumpAndSettle();

    expect(workspace.interface.retimeOpensToSpeed, isFalse);
    expect(workspace.interface.videoAsSequenceLayer, isFalse);
    expect(workspace.firstRunDone, isTrue);
  });

  testWidgets('skipping keeps the defaults and still counts as answered',
      (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('first-run-skip')));
    await tester.pumpAndSettle();

    expect(workspace.interface.retimeOpensToSpeed, isFalse);
    expect(workspace.firstRunDone, isTrue);
  });

  testWidgets('the update tick is on, and the answer carries it', (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();

    // Ticked before anything is touched (K-296): the default is that Lumit
    // looks for new versions.
    await tester.tap(find.byKey(const ValueKey('first-run-ae')));
    await tester.pumpAndSettle();
    expect(workspace.autoUpdate, isTrue);
  });

  testWidgets('unticking the update box is remembered', (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('first-run-auto-update')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('first-run-vegas')));
    await tester.pumpAndSettle();

    expect(workspace.autoUpdate, isFalse);
    // The editing answer is unaffected: two questions, one screen.
    expect(workspace.interface.videoAsSequenceLayer, isTrue);
  });

  testWidgets('skipping leaves update checks on', (tester) async {
    await tester.pumpWidget(host());
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('first-run-skip')));
    await tester.pumpAndSettle();
    expect(workspace.autoUpdate, isTrue);
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
