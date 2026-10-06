// The cache bar: the stripe under the time ruler showing which frames are held
// (docs/07-UI-SPEC.md §3.2, docs/06-RENDER-PIPELINE.md §5.6).
//
// The run collapsing is a pure function and tested as one. What it draws is
// tested against the real engine, because the question the bar answers — "does
// this frame play now?" — is the engine's to answer and was not previously
// askable at all: the bridge reported only totals.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import 'frb_test_support.dart';

/// A postage-stamp composition with one solid in it: small enough that every
/// render in these tests is trivial even on a software rasteriser, which is
/// what the CI runner has.
CompositionReference _stampComp(ProjectReference project, String name,
    {BridgeRational? duration}) {
  final comp = project.newComposition(name: name);
  final was = comp.getSettings();
  comp.setSettings(
    settings: BridgeCompSettings(
      name: was.name,
      width: 160,
      height: 90,
      fpsNum: was.fpsNum,
      fpsDen: was.fpsDen,
      duration: duration ?? was.duration,
      background: was.background,
      shutterAngle: was.shutterAngle,
      motionBlurSamples: was.motionBlurSamples,
    ),
  );
  comp.addSolidLayer();
  return comp;
}

void main() {
  group('Cache bar runs', () {
    test('uncached frames are gaps, not runs', () {
      expect(cacheBarRuns([0, 2, 2, 0, 2]), [(1, 3, 2), (4, 5, 2)]);
    });
  });

  group('Cache bar against the engine', () {
    setUpAll(initEngineForTests);

    // Viewer frames only ever cross as GPU handles now, so nothing the
    // Viewer shows leaves bytes behind — the rendered-frame cache is filled by
    // the scope path, which needs CPU pixels and files what it renders.

    /// The whole point of the bar: a frame that has been rendered (here, for a
    /// trace) reads back as held, and one that has not reads as nothing.
    testWidgets('a rendered frame shows as held, an unrendered one does not',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      expect(
        comp.cachedFrames(frames: BigInt.from(8), scale: 1.0),
        everyElement(0),
        reason: 'nothing rendered yet',
      );

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();
      // Retried, because the cache is process-global and every parallel test
      // suite's committed edit invalidates it: a hold observed and then
      // snatched away by a neighbour's commit is the environment, not the
      // regression this test exists for.
      late List<int> tiers;
      for (var attempt = 0; attempt < 5; attempt++) {
        comp.renderScope(
          frame: BigInt.zero,
          scale: p.uiState.viewerScale,
          kind: 0,
          colours: scopeColoursFor(LumitTheme.dark()),
        );
        await settleFrb(
          tester,
          minRounds: 20,
          maxRounds: 200,
          until: () =>
              cacheStorageOf(comp.cachedFrames(
                  frames: BigInt.from(8), scale: p.uiState.viewerScale)[0]) !=
              0,
        );
        tiers = comp
            .cachedFrames(frames: BigInt.from(8), scale: p.uiState.viewerScale)
            .map(cacheStorageOf)
            .toList();
        if (tiers[0] != 0) break;
      }
      expect(tiers[0], 2, reason: 'the frame under the playhead is held');
      // The other half of the name — "an unrendered one does not" — is the
      // `everyElement(0)` above, taken before anything was rendered. It cannot
      // be asserted again down here: the idle fill works outwards from the
      // anchor, two frames ahead for every one behind, for as long as the
      // settle loop keeps turning (docs/06 §5.5, and the sibling test that pins
      // that behaviour). So which neighbours are still cold at this instant is
      // a race between the fill and the assertion — one the owner's machine
      // happened to win and the Linux runner lost, which makes it a statement
      // about timing rather than about the bar.
    });

    /// A composition far longer than the panel is wide gives a run whose right
    /// edge lands past the bar. `num.clamp` throws when the lower bound exceeds
    /// the upper, so the naive clamp crashed the paint outright.
    testWidgets('a run at the far end of a long comp does not crash the paint',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Long');
      final settings = comp.getSettings();
      comp.setSettings(
        settings: BridgeCompSettings(
          name: settings.name,
          width: settings.width,
          height: settings.height,
          fpsNum: settings.fpsNum,
          fpsDen: settings.fpsDen,
          // 4000 frames at the comp's 60 fps.
          background: settings.background,
          shutterAngle: settings.shutterAngle,
          motionBlurSamples: settings.motionBlurSamples,
          duration: const BridgeRational(num: 200, den: 3),
        ),
      );
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1000, 500),
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 10, maxRounds: 60);

      expect(tester.takeException(), isNull,
          reason: '4000 frames across 1000 px must not throw in paint');
    });

    /// **Fronting a composition asks for its picture.** Nothing else does: the
    /// playhead has not moved and no edit has landed, so before this the Viewer
    /// kept the previous comp's frame and the engine's idle fill — anchored on
    /// the frame last *shown* — banked nothing for the new comp until some edit
    /// happened to ask for a frame. Asserted through the fill, because the fill
    /// is the visible consequence and needs no GPU export to observe.
    testWidgets('fronting a composition warms it without an edit',
        (tester) async {
      final p = freshProject();
      final first = _stampComp(p.state.project!, 'First');
      final second = _stampComp(p.state.project!, 'Second');
      p.uiState.setSelectedComp(first);

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      // Front the other one, exactly as the Timeline's tab bar does — no edit,
      // no playhead move.
      p.uiState.setSelectedComp(second);
      await tester.pump();

      await tester.runAsync(() async {
        for (var i = 0; i < 150; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          final tiers = second
              .cachedFrames(frames: BigInt.from(8), scale: 1.0)
              .map(cacheStorageOf)
              .toList();
          if (tiers[0] == 2) return;
        }
        fail('fronting the composition never asked for a frame of it');
      });
    });

    /// **An undo comes back to a warm cache.** This is the other half of
    /// content keying, and the one a user feels most: make a change, dislike it,
    /// undo — and the frames from before the change are still filed under the
    /// names the restored document asks for, so nothing has to be rendered again.
    ///
    /// Under positional keying both the edit and the undo emptied the cache, so
    /// an undo meant caching the whole work area from scratch. There is no
    /// counter for "did not re-render" — the observable is that the bar is green
    /// again immediately, without waiting for a fill.
    ///
    /// (This replaces a test that guarded a race the design has removed: a
    /// commit landing while the worker was parked used to be served from the
    /// caches that commit had retired. With nothing retired on commit, there is
    /// no wrong side of the invalidation to be on.)
    testWidgets('an undo finds its frames still held', (tester) async {
      final p = freshProject();
      final comp = _stampComp(p.state.project!, 'Scene',
          duration: const BridgeRational(num: 1, den: 3));
      final layer = comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();
      List<int> tiers() => comp
          .cachedFrames(frames: BigInt.from(4), scale: p.uiState.viewerScale)
          .map(cacheStorageOf)
          .toList();
      await settleFrb(
        tester,
        minRounds: 20,
        maxRounds: 400,
        until: () => tiers()[0] != 0,
      );
      expect(tiers()[0], 2, reason: 'the shown frame is held');

      // A real edit: the picture changes, so the frame is renamed and the bar
      // goes cold for it (nothing was thrown away — the old frame is still
      // there under its own name, which is the next assertion).
      layer.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
      await settleFrb(tester, minRounds: 20, maxRounds: 200);

      p.state.project!.undo();
      await settleFrb(tester, minRounds: 20, maxRounds: 200);
      expect(
        tiers()[0],
        2,
        reason: 'the undone document asks for the name it asked for before, '
            'and the frame is still held',
      );
    });

    /// **A budget set before the worker existed still reaches the cache.** The
    /// worker seeded "what I have applied" from the wish itself, and a fresh
    /// renderer's cache holds the built-in default — so a budget restored at
    /// launch (or left behind by the previous project) was recorded as applied
    /// without ever being applied, and the cache stayed at 512 MiB all session
    /// while Settings read whatever the user chose. The meter reports the
    /// budget the cache actually holds to, which is what makes this askable.
    testWidgets('the VRAM budget reaches the cache, whenever it was set',
        (tester) async {
      // Set before anything renders, exactly as the settings restore does.
      const wanted = 1 << 30; // 1 GiB, and not the default
      setVramCacheBudget(bytes: BigInt.from(wanted));

      final p = freshProject();
      final comp = _stampComp(p.state.project!, 'Scene');
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      await tester.runAsync(() async {
        for (var i = 0; i < 60; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          if (vramCacheStats().budgetBytes.toInt() == wanted) return;
        }
        fail('the cache is holding to ${vramCacheStats().budgetBytes} bytes, '
            'not the $wanted asked for');
      });
    });

    /// The idle fill: show a frame, leave the engine alone for a moment, and it
    /// banks the frames around the playhead on its own — forward-biased, so the
    /// ones ahead come first. Real wall-clock waits, because the worker is a
    /// real thread with a real 200 ms lull gate; without the fill this times
    /// out with nothing held but the shown frame.
    testWidgets('the idle fill warms frames around the playhead',
        (tester) async {
      final p = freshProject();
      // Nothing measured here: a measured frame is deliberately composited
      // rather than served from a tier, which is the opposite of what
      // this test is about and enough extra work under a loaded runner to eat
      // the fill's window.
      p.uiState.renderTimings.setMeasuring(false);
      addTearDown(() => p.uiState.renderTimings.setMeasuring(true));
      final comp = p.state.project!.newComposition(name: 'Scene');
      // A postage-stamp comp, because the question is whether the fill *banks*
      // frames, not how fast a machine can composite one. At the default
      // 1920×1080 this waited on three real 2-megapixel composites, which the
      // CI runner does on a software rasteriser: it ran out of patience there
      // and failed as though the fill were broken. Shrinking the picture makes
      // each fill render trivial on any machine, and changes nothing about the
      // behaviour being pinned.
      final was = comp.getSettings();
      comp.setSettings(
        settings: BridgeCompSettings(
          name: was.name,
          width: 160,
          height: 90,
          fpsNum: was.fpsNum,
          fpsDen: was.fpsDen,
          duration: was.duration,
          background: was.background,
          shutterAngle: was.shutterAngle,
          motionBlurSamples: was.motionBlurSamples,
        ),
      );
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      comp.renderFrame(
        frame: BigInt.from(5),
        scale: 1.0,
        mode: BridgePlaybackMode.everyFrame,
        view: 0,
      );

      // Fifteen seconds of patience, not five: the first render of a session
      // also builds the renderer and compiles its shaders, which on a software
      // adapter is seconds by itself. A generous ceiling costs nothing when the
      // fill works — the loop returns the moment it does.
      await tester.runAsync(() async {
        for (var i = 0; i < 150; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          final tiers = comp
              .cachedFrames(frames: BigInt.from(12), scale: 1.0)
              .map(cacheStorageOf)
              .toList();
          // Ahead of the playhead fills first (two forward for one back),
          // but all three neighbours arriving is the honest "it works".
          if (tiers[6] == 2 && tiers[7] == 2 && tiers[4] == 2) return;
        }
        fail('the idle fill banked nothing around the playhead');
      });
    });
  }, skip: !engineAvailable);
}
