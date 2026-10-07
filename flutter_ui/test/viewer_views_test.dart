// Several Viewers, and several views inside one (docs/impl/multi-viewer.md).
//
// The dock half — a panel that can be in the arrangement more than once — and
// the view model half: what each view shows, its lock, where an opened item
// lands, and how both halves persist.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeRational;
import 'package:lumit_flutter/src/rust/api/footage.dart' show BridgeMediaInfo;
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/ui_state.dart';
import 'package:lumit_flutter/state/viewer_views.dart';

void main() {
  group('the dock holds more than one Viewer', () {
    test('an arrangement written before panes had numbers reads unchanged', () {
      // Exactly the bytes an older build wrote: no `n` anywhere.
      final old = {
        'kind': 'split',
        'axis': 'horizontal',
        'shares': [0.5, 0.5],
        'children': [
          {'kind': 'pane', 'panel': 'viewer'},
          {'kind': 'pane', 'panel': 'timeline'},
        ],
      };
      final read = DockNode.fromJson(old)! as DockSplit;
      expect(panesIn(read), [
        (panel: Panel.viewer, instance: 0),
        (panel: Panel.timeline, instance: 0),
      ]);
      // And it writes back the same bytes: instance 0 is left out, so a
      // workspace with one Viewer is untouched by any of this.
      expect(read.toJson(), old);
    });
  });

  group('views', () {
    ViewerViews viewsWith(int count) {
      final views = ViewerViews();
      views.paneLayouts[Panel.viewer.pane()] = switch (count) {
        1 => ViewLayout.one,
        2 => ViewLayout.twoAcross,
        _ => ViewLayout.four,
      };
      views.forPane(Panel.viewer.pane());
      return views;
    }

    test('a pane makes the views its layout asks for, and no more', () {
      final views = viewsWith(4);
      expect(views.forPane(Panel.viewer.pane()).length, 4);
      views.setLayout(Panel.viewer.pane(), ViewLayout.one);
      expect(views.forPane(Panel.viewer.pane()).length, 1);
      expect(views.views.length, 1, reason: 'the closed views are let go of');
      expect(views.takeClosed().length, 3,
          reason: 'and the engine is told, so their textures go with them');
    });

    test('a locked active view is not stolen', () {
      final views = viewsWith(2);
      final pair = views.forPane(Panel.viewer.pane());
      views.front(pair[0].id);
      pair[0].locked = true;
      expect(views.viewForOpening(Panel.viewer.pane())?.id, pair[1].id,
          reason: 'it lands in the other, unlocked view');
    });

    /// A file has no transport of its own, and the composition being played
    /// must not be painted over the clip somebody is looking at.
    test('the transport never plays into a view of a file', () {
      final views = viewsWith(2);
      final pair = views.forPane(Panel.viewer.pane());
      pair[0].mode = ViewMode.footage;
      pair[0].itemId = 'item-a';
      views.front(pair[0].id);
      expect(views.previewing?.id, pair[1].id,
          reason: 'the picture goes to the composition view beside it');

      pair[1].mode = ViewMode.footage;
      expect(views.previewing?.id, pair[0].id,
          reason: 'with nowhere better it is the active one, as before');
    });

    test('both halves round-trip, and each drops what the other does not name',
        () {
      final views = viewsWith(2);
      final pair = views.forPane(Panel.viewer.pane());
      pair[0].compId = 'comp-a';
      pair[1].compId = 'comp-b';
      pair[1].locked = true;
      pair[1].magnification = 2.0;
      pair[1].look = (stops: -1.25, toneMap: true);
      pair[1].region = [0.1, 0.2, 0.8, 0.9];
      views.front(pair[1].id);
      views.alwaysPreviewId = pair[0].id;
      views.setCompare(Panel.viewer.pane(), CompareMode.wipe);
      views.setDivider(Panel.viewer.pane(), 0.3);

      final project = views.toProjectJson();
      final workspace = views.toWorkspaceJson();

      final back = ViewerViews()..restore(project, workspace);
      final read = back.forPane(Panel.viewer.pane());
      expect(read.length, 2);
      expect(read[0].compId, 'comp-a');
      expect(read[1].compId, 'comp-b');
      expect(read[1].locked, isTrue);
      expect(read[1].magnification, 2.0);
      expect(read[1].look, (stops: -1.25, toneMap: true));
      expect(read[1].region, [0.1, 0.2, 0.8, 0.9]);
      expect(back.activeId, read[1].id);
      expect(back.alwaysPreviewId, read[0].id);
      expect(back.compareOf(Panel.viewer.pane()), CompareMode.wipe);
      expect(back.dividerOf(Panel.viewer.pane()), 0.3);

      // **A workspace someone sent you carries no locks.** The project half
      // is what says what a view shows; without it the views are laid out and
      // bound to nothing.
      final shared = ViewerViews()..restore(null, workspace);
      expect(shared.views, isEmpty,
          reason: 'a view the project does not describe is not restored');

      // And a project half nothing lays out is dropped rather than kept
      // invisible.
      final orphaned = ViewerViews()..restore(project, null);
      expect(orphaned.views, isEmpty);
    });

    test('a malformed record opens as a default rather than failing', () {
      final views = ViewerViews()
        ..restore(
          [
            {'id': 'a', 'zoom': 'not a number', 'region': [1, 2]},
            {'no id': true},
            'not a map',
          ],
          {
            'panes': {'viewer:0': ['a']},
            'layouts': {'viewer:0': 'nonsense'},
          },
        );
      expect(views.views.length, 1);
      expect(views.views.first.magnification, isNull);
      expect(views.views.first.region, isNull);
      expect(views.layoutOf(Panel.viewer.pane()), ViewLayout.one);
    });
  });

  /// How long a footage view's item is, in its own frames: what the render
  /// request carries, because the frontend is the side that has probed the
  /// file (docs/impl/multi-viewer.md §3.6).
  group('a footage view counts the frames of its item', () {
    BridgeMediaInfo facts({
      required int fpsNum,
      required int fpsDen,
      required int seconds,
      int den = 1,
    }) =>
        BridgeMediaInfo(
          width: 1920,
          height: 1080,
          fpsNum: fpsNum,
          fpsDen: fpsDen,
          duration: BridgeRational(num: seconds, den: den),
          channels: 0,
          sampleRate: 0,
          isStill: false,
        );

    test('a length at a rate is that many frames', () {
      expect(
          LumitUiState.framesOf(facts(fpsNum: 25, fpsDen: 1, seconds: 4)), 100);
      // 24000/1001 over five seconds: the rate that is not a whole number.
      expect(
          LumitUiState.framesOf(
              facts(fpsNum: 24000, fpsDen: 1001, seconds: 5)),
          119);
    });
  });
}
