// The custom fade editor (docs/impl/audio-timeline.md §3): a crossfade opens
// with both curves and the Keep level box, a lone fade opens with neither, and
// a preset pressed there lands on both sides when Apply is pressed.
//
// The curve drawing itself is checked in clip_fades_test.dart, where it needs
// no widget tree.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/clip_fade_popover.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  late BuildContext host;

  Widget harness() => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(builder: (context) {
                host = context;
                return const SizedBox.expand();
              }),
            ],
          ),
        ),
      );

  tearDown(closeLumitPopups);

  testWidgets('a crossfade opens with both curves and Keep level',
      (tester) async {
    await tester.pumpWidget(harness());
    BridgeClipFadeShape? outgoing;
    BridgeClipFadeShape? incoming;
    showClipFadePopover(
      context: host,
      position: const Offset(200, 200),
      outgoing: const BridgeClipFadeShape.fast(),
      incoming: const BridgeClipFadeShape.fast(),
      onApply: (out, into) {
        outgoing = out;
        incoming = into;
      },
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey<String>('atl-fade-box')), findsOneWidget);
    expect(find.byKey(const ValueKey<String>('atl-fade-keep')), findsOneWidget,
        reason: 'a join can be kept level; there are two curves to keep');

    await tester
        .tap(find.byKey(const ValueKey<String>('atl-fade-preset-slow')));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey<String>('atl-fade-apply')));
    await tester.pumpAndSettle();

    expect(outgoing, const BridgeClipFadeShape.slow());
    expect(incoming, const BridgeClipFadeShape.slow(),
        reason: 'a preset is the pair of shapes the join takes');
  });

  testWidgets('a lone fade opens with one curve and no Keep level',
      (tester) async {
    await tester.pumpWidget(harness());
    BridgeClipFadeShape? outgoing;
    BridgeClipFadeShape? incoming;
    showClipFadePopover(
      context: host,
      position: const Offset(200, 200),
      outgoing: null,
      incoming: const BridgeClipFadeShape.linear(),
      onApply: (out, into) {
        outgoing = out;
        incoming = into;
      },
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey<String>('atl-fade-keep')), findsNothing,
        reason: 'there is nothing on the other side to keep level against');

    await tester.tap(find.byKey(const ValueKey<String>('atl-fade-apply')));
    await tester.pumpAndSettle();

    expect(outgoing, isNull,
        reason: 'the side that was not there is not written');
    expect(incoming, const BridgeClipFadeShape.linear());
  });
}
