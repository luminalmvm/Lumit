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

    test('a second Viewer round-trips, and names itself', () {
      final root = DockSplit(
        DockAxis.horizontal,
        [DockPane(Panel.viewer), DockPane(Panel.timeline)],
        [0.5, 0.5],
      );
      final made = addPane(root, Panel.viewer, beside: Panel.viewer.pane());
      expect(made, (panel: Panel.viewer, instance: 1));
      expect(panelsIn(root).where((p) => p == Panel.viewer).length, 2);

      final json = root.toJson();
      expect(
        (json['children'] as List).any((c) => (c as Map)['n'] == 1),
        isTrue,
        reason: 'the second pane writes its number',
      );
      final read = DockNode.fromJson(json)! as DockSplit;
      expect(panesIn(read), panesIn(root));
    });

    test('the two Viewers move and close on their own', () {
      final root = DockSplit(
        DockAxis.horizontal,
        [DockPane(Panel.viewer), DockPane(Panel.timeline)],
        [0.5, 0.5],
      );
      final second = addPane(root, Panel.viewer, beside: Panel.viewer.pane());

      // Dragging one onto the Timeline stacks that one and leaves the other.
      movePanel(root, second, Panel.timeline.pane(), DropPosition.stack);
      expect(panesIn(root).toSet(),
          {Panel.viewer.pane(), second, Panel.timeline.pane()});

      // Closing the second leaves the first where it was.
      closePane(root, second);
      expect(panesIn(root).toSet(),
          {Panel.viewer.pane(), Panel.timeline.pane()});
    });

    test('the next instance is free even after the middle one closes', () {
      final root = DockSplit(DockAxis.horizontal, [DockPane(Panel.viewer)], [1]);
      final b = addPane(root, Panel.viewer);
      final c = addPane(root, Panel.viewer);
      expect([b.instance, c.instance], [1, 2]);
      closePane(root, b);
      // Not 1 again: an id that came back would re-point anything holding it.
      expect(addPane(root, Panel.viewer).instance, 3);
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

    test('a split seeds the new view with what the first is showing', () {
      final views = viewsWith(1);
      views.views.first.compId = 'comp-a';
      views.setLayout(Panel.viewer.pane(), ViewLayout.twoAcross);
      final pair = views.forPane(Panel.viewer.pane());
      expect(pair[1].compId, 'comp-a');
      expect(pair[1].id, isNot(pair[0].id));
      expect(pair[1].engineId, isNot(pair[0].engineId));
    });

    test('an opened item lands in the active view when it is unlocked', () {
      final views = viewsWith(2);
      final pair = views.forPane(Panel.viewer.pane());
      views.front(pair[1].id);
      expect(views.viewForOpening(Panel.viewer.pane())?.id, pair[1].id);
    });

    test('a locked active view is not stolen', () {
      final views = viewsWith(2);
      final pair = views.forPane(Panel.viewer.pane());
      views.front(pair[0].id);
      pair[0].locked = true;
      expect(views.viewForOpening(Panel.viewer.pane())?.id, pair[1].id,
          reason: 'it lands in the other, unlocked view');
    });

    test('every view locked and a layout with room opens another view', () {
      final views = viewsWith(4);
      final all = views.forPane(Panel.viewer.pane());
      for (final v in all) {
        v.locked = true;
      }
      // Take the fourth out of the layout, leaving room the way a pane whose
      // layout was just grown has it.
      views.paneViews[Panel.viewer.pane()]!.removeLast();
      views.views.removeWhere((v) => v.id == all.last.id);
      final into = views.viewForOpening(Panel.viewer.pane());
      expect(into, isNotNull);
      expect(into!.locked, isFalse);
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

    test('every view locked and no room says so, for the caller to open a pane',
        () {
      final views = viewsWith(2);
      for (final v in views.forPane(Panel.viewer.pane())) {
        v.locked = true;
      }
      expect(views.viewForOpening(Panel.viewer.pane()), isNull,
          reason: 'the caller opens another Viewer panel rather than this '
              'growing a layout nobody asked for');
    });

    test('two views on one composition are looked at differently', () {
      final views = viewsWith(2);
      final pair = views.forPane(Panel.viewer.pane());
      pair[0].magnification = 1.0;
      pair[1].magnification = 4.0;
      pair[0].look = (stops: 0.0, toneMap: false);
      pair[1].look = (stops: 1.5, toneMap: true);
      expect(views.optionsFor(pair[0]).magnification, 1.0);
      expect(views.optionsFor(pair[1]).magnification, 4.0);

      // With the share switch on, every view reads the active one's.
      views.front(pair[0].id);
      views.shareViewOptions = true;
      expect(views.optionsFor(pair[1]).magnification, 1.0);
      expect(views.optionsFor(pair[1]).look.stops, 0.0);
      // And turning it off leaves each holding what it was showing.
      views.shareViewOptions = false;
      expect(views.optionsFor(pair[1]).magnification, 4.0);
    });

    test('cycling walks the views and comes round', () {
      final views = viewsWith(4);
      final all = views.forPane(Panel.viewer.pane());
      views.front(all.first.id);
      views.cycle(1);
      expect(views.activeId, all[1].id);
      views.cycle(-1);
      expect(views.activeId, all[0].id);
      views.cycle(-1);
      expect(views.activeId, all.last.id, reason: 'it wraps');
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

    test('a still has one frame, not none', () {
      expect(LumitUiState.framesOf(facts(fpsNum: 25, fpsDen: 1, seconds: 0)), 1,
          reason: 'a picture with no length is still a picture');
    });
  });
}
