// Movable, resizable modal windows (K-242): dragging moves one, the corner grip
// resizes one, and both are remembered in the workspace store so the window
// opens where it was left — this session and the next.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  late Workspace workspace;

  setUp(() {
    // Never the developer's own settings file.
    Workspace.storeOverride =
        '${Directory.systemTemp.path}/lumit-modal-window-test.json';
    workspace = Workspace();
    modalPlacementStore = workspace;
  });

  tearDown(() {
    modalPlacementStore = null;
    Workspace.storeOverride = null;
  });

  /// An app around a button that opens one modal window.
  Widget host({String? id, Size? initialSize}) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(
                builder: (context) => Center(
                  child: GestureDetector(
                    key: const ValueKey('open'),
                    onTap: () => showLumitModal<void>(
                      context: context,
                      id: id,
                      initialSize: initialSize,
                      minSize: const Size(100, 80),
                      builder: (_) => const FloatSurface(
                        child: SizedBox.expand(
                          child: Text('body', key: ValueKey('body')),
                        ),
                      ),
                    ),
                    child: const SizedBox(
                      width: 40,
                      height: 20,
                      child: Text('open'),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      );

  Future<void> open(WidgetTester tester) async {
    await tester.tap(find.byKey(const ValueKey('open')));
    await tester.pump();
  }

  final body = find.byKey(const ValueKey('body'));
  final grip = find.byKey(const ValueKey('window-resize-grip'));

  testWidgets('a window opens centred and drags where it is put',
      (tester) async {
    await tester.pumpWidget(host(id: 'test-window'));
    await open(tester);

    final centred = tester.getCenter(body);
    await tester.drag(body, const Offset(60, -40));
    await tester.pump();

    expect(tester.getCenter(body) - centred, const Offset(60, -40));
  });

  testWidgets('where it was left is remembered, and reopening lands there',
      (tester) async {
    await tester.pumpWidget(host(id: 'test-window'));
    await open(tester);
    final centred = tester.getCenter(body);
    await tester.drag(body, const Offset(50, 30));
    await tester.pump();

    expect(workspace.windowPlacements['test-window']?.offset,
        const Offset(50, 30));

    // Close it — the backdrop is the click-outside dismissal — and reopen.
    await tester.tapAt(const Offset(5, 5));
    await tester.pump();
    expect(body, findsNothing);
    await open(tester);

    expect(tester.getCenter(body) - centred, const Offset(50, 30),
        reason: 'it opens where it was left');
  });

  testWidgets('a window with no id is not remembered', (tester) async {
    await tester.pumpWidget(host());
    await open(tester);
    await tester.drag(body, const Offset(20, 20));
    await tester.pump();

    expect(workspace.windowPlacements, isEmpty);
  });

  testWidgets('the corner grip resizes, and the top-left edge stays put',
      (tester) async {
    await tester.pumpWidget(
        host(id: 'sized-window', initialSize: const Size(300, 200)));
    await open(tester);

    final before = tester.getRect(find.byType(FloatSurface));
    expect(before.size, const Size(300, 200));

    await tester.drag(grip, const Offset(80, 60));
    await tester.pump();

    final after = tester.getRect(find.byType(FloatSurface));
    expect(after.size, const Size(380, 260));
    expect(after.topLeft, before.topLeft,
        reason: 'dragging the bottom-right corner moves only that corner');
    expect(workspace.windowPlacements['sized-window']?.size,
        const Size(380, 260));
  });

  testWidgets('a size cannot go below the minimum or past the app window',
      (tester) async {
    await tester.pumpWidget(
        host(id: 'sized-window', initialSize: const Size(300, 200)));
    await open(tester);

    await tester.drag(grip, const Offset(-900, -900));
    await tester.pump();
    expect(tester.getSize(find.byType(FloatSurface)), const Size(100, 80));

    await tester.drag(grip, const Offset(9000, 9000));
    await tester.pump();
    final screen = tester.getSize(find.byType(Overlay));
    expect(tester.getSize(find.byType(FloatSurface)), screen);
  });

  /// The panels hang their keyboard commands off the hardware keyboard, so
  /// they have to be told a window is up (K-243) — and told it is gone again
  /// however it left, including having its tree taken down under it. A count
  /// that stuck above zero would leave the keyboard dead for the session.
  testWidgets('a window says it is open, and stops when it goes',
      (tester) async {
    // A sized window, so the corner of the screen is the backdrop to click.
    await tester.pumpWidget(
        host(id: 'test-window', initialSize: const Size(200, 150)));
    expect(lumitModalOpen, isFalse);

    await open(tester);
    expect(lumitModalOpen, isTrue);

    await tester.tapAt(const Offset(5, 5));
    await tester.pumpAndSettle();
    expect(body, findsNothing, reason: 'the window went');
    expect(lumitModalOpen, isFalse, reason: 'dismissing closed it');

    await open(tester);
    expect(lumitModalOpen, isTrue);
    await tester.pumpWidget(const SizedBox());
    expect(lumitModalOpen, isFalse,
        reason: 'and so did the tree going away under it');
  });

  testWidgets('a placement survives a save and load of the store',
      (tester) async {
    await tester.pumpWidget(
        host(id: 'sized-window', initialSize: const Size(300, 200)));
    await open(tester);
    await tester.drag(body, const Offset(25, 15));
    await tester.pump();
    await tester.drag(grip, const Offset(40, 40));
    await tester.pump();

    final reloaded = Workspace()..load();
    final placement = reloaded.windowPlacements['sized-window'];
    expect(placement?.offset, workspace.windowPlacements['sized-window']?.offset);
    expect(placement?.size, const Size(340, 240));
  });
}
