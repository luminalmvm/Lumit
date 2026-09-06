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
    test('contiguous frames of one tier collapse to a single run', () {
      expect(cacheBarRuns([2, 2, 2]), [(0, 3, 2)]);
    });

    test('uncached frames are gaps, not runs', () {
      expect(cacheBarRuns([0, 2, 2, 0, 2]), [(1, 3, 2), (4, 5, 2)]);
    });

    /// A frame held only at a coarser resolution is a different state, so it
    /// cannot be merged into the run beside it.
    test('a change of tier breaks the run', () {
      expect(cacheBarRuns([2, 2, 1, 1, 2]), [(0, 2, 2), (2, 4, 1), (4, 5, 2)]);
    });

    test('nothing held draws nothing', () {
      expect(cacheBarRuns([0, 0, 0]), isEmpty);
      expect(cacheBarRuns([]), isEmpty);
    });

    /// A run breaks on the WHOLE byte, so a stretch held at half and a stretch
    /// held at quarter are two runs even though both are "held" — they are
    /// drawn at different strengths, and merging them would say the whole
    /// stretch was the finer of the two.
    test('a change of resolution tier breaks the run too', () {
      const heldFull = 0x12;
      const heldHalf = 0x22;
      expect(cacheBarRuns([heldFull, heldFull, heldHalf]),
          [(0, 2, heldFull), (2, 3, heldHalf)]);
    });
  });

  group('Cache bar strip bytes', () {
    /// The two nibbles, split the same way the painter splits them and the
    /// same way `framecache::bar::storage_of` splits them in Rust.
    test('a strip byte carries the storage state and the resolution tier', () {
      expect(cacheStorageOf(0x12), 2, reason: 'held at the shown resolution');
      expect(cacheDivisorOf(0x12), 1, reason: 'and made at full size');
      expect(cacheStorageOf(0x43), 3, reason: 'parked on disk, coarser');
      expect(cacheDivisorOf(0x43), 4, reason: 'a quarter of the shown size');
      expect(cacheStorageOf(0), 0, reason: 'nothing held has no state');
      expect(cacheDivisorOf(0), 0, reason: 'and no size');
    });

    /// §6.3, and the one thing the design table did not name: the realtime
    /// controller renders a **third** as well as a half and a quarter, so
    /// there are four tiers and three steps. A third takes the coarsest step,
    /// which under-promises rather than telling anyone it is finer than it is;
    /// a divisor a later engine invents lands there too.
    test('the coarsest step means a third or a quarter, and anything stranger',
        () {
      expect(cacheTierOpacity(1), 1.0);
      expect(cacheTierOpacity(2), 0.7);
      expect(cacheTierOpacity(3), cacheTierOpacity(4),
          reason: 'a third is drawn as coarsely as a quarter, never finer');
      expect(cacheTierOpacity(4), 0.4);
      expect(cacheTierOpacity(9), 0.4,
          reason: 'a divisor this build does not know is coarser, not finer');
    });

    /// Never colour alone (docs/15 §11). A bar that said "coarser" with a
    /// shade alone read as one green changing at random — the tone is a real
    /// distinction, but a 3px stripe is not where a shade is legible. Coarser
    /// draws shorter as well, and the two scales agree step for step: whatever
    /// is fainter is never taller.
    test('a coarser tier is drawn shorter as well as fainter', () {
      expect(cacheTierHeight(1), 1.0, reason: 'a full frame fills the bar');
      expect(cacheTierHeight(2), lessThan(cacheTierHeight(1)));
      expect(cacheTierHeight(3), cacheTierHeight(4),
          reason: 'a third is drawn as coarsely as a quarter, never finer');
      expect(cacheTierHeight(9), cacheTierHeight(4),
          reason: 'a divisor this build does not know is coarser, not finer');
      for (final divisor in [1, 2, 3, 4, 9]) {
        expect(cacheTierHeight(divisor), greaterThan(0),
            reason: 'a held frame is always visible, however coarse');
        expect(cacheTierHeight(divisor), lessThanOrEqualTo(1.0));
      }
      // The two readings of "coarser" must not disagree: a run drawn fainter
      // and taller would say two different things at once.
      for (final pair in [(1, 2), (2, 3), (2, 4)]) {
        expect(cacheTierHeight(pair.$2), lessThan(cacheTierHeight(pair.$1)),
            reason: 'divisor ${pair.$2} is coarser than ${pair.$1}');
        expect(cacheTierOpacity(pair.$2), lessThan(cacheTierOpacity(pair.$1)),
            reason: 'and fainter, in the same direction');
      }
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

    /// **An edit that cannot change a pixel must not empty the bar.**
    ///
    /// A rename is the case that used to show what positional keying cost:
    /// renaming a layer changes no pixel, and every held frame of the
    /// composition was retired anyway, so the stripe went blank on a keystroke.
    /// Frames are named by a hash of their content now, so the rename produces
    /// the same names and the bar stays exactly as it was — which is the
    /// behaviour this whole tier stack exists for. Fails against the old
    /// invalidate-everything commit hook.
    testWidgets('a rename leaves the bar alone', (tester) async {
      final p = freshProject();
      final comp = _stampComp(p.state.project!, 'Scene');
      final layer = comp.addSolidLayer();
      final other = p.state.project!.newComposition(name: 'Other');
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();
      comp.renderScope(
        frame: BigInt.zero,
        scale: p.uiState.viewerScale,
        kind: 0,
        colours: scopeColoursFor(LumitTheme.dark()),
      );
      await settleFrb(
        tester,
        minRounds: 20,
        maxRounds: 400,
        until: () =>
            cacheStorageOf(comp.cachedFrames(
                frames: BigInt.from(4), scale: p.uiState.viewerScale)[0]) !=
            0,
      );
      expect(
        cacheStorageOf(comp.cachedFrames(
            frames: BigInt.from(4), scale: p.uiState.viewerScale)[0]),
        2,
      );

      layer.rename(name: 'Renamed');
      // The strip is a published mirror the worker refreshes, so give it a turn
      // to recompute after the commit — the point being that when it does, the
      // frame is still held.
      await settleFrb(tester, minRounds: 20, maxRounds: 200);
      expect(
        cacheStorageOf(comp.cachedFrames(
            frames: BigInt.from(4), scale: p.uiState.viewerScale)[0]),
        2,
        reason: 'a rename cannot change a pixel, so the frame is still held',
      );
      expect(
        other.cachedFrames(frames: BigInt.from(4), scale: 1.0),
        everyElement(0),
        reason: 'and the other composition never held any',
      );
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

    /// A scope trace needs CPU pixels, and the zero-copy Viewer keeps none —
    /// so the first trace of a frame renders and files it, and a second trace
    /// of the same frame is served from the cache rather than compositing the
    /// composition again.
    testWidgets('a second trace of the same frame is served from the cache',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      // A one-frame work area, so the worker's idle fill has nothing to make
      // while this test is looking. The entry count below is the whole
      // memory tier's, and a fill frame the card pushed out landed in it
      // between the two readings on the Linux runner, reading as a second
      // composite that never happened.
      comp.setWorkArea(
        span: BridgeSpan(
          inPoint: comp.timeOfFrame(frame: 0),
          outPoint: comp.timeOfFrame(frame: 1),
          startOffset: const BridgeRational(num: 0, den: 1),
        ),
      );
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      final base = cacheStats().entries;
      comp.renderScope(
        frame: BigInt.zero,
        scale: p.uiState.viewerScale,
        kind: 0,
        colours: scopeColoursFor(LumitTheme.dark()),
      );
      // Wait for THIS comp's frame to be held, not for the process-wide entry
      // count to be non-zero — earlier tests in this file leave residue in the
      // shared cache, and a `before` snapshotted on their entries races the
      // first trace (two queued traces collapse to the newest, so the first
      // can vanish entirely). And for the memory tier to have gained an entry
      // since the trace was asked for: the Viewer puts the frame on the card
      // first, which the strip reports as held, and a `before` read then saw
      // the first trace's own filing land as though the second had composited.
      await settleFrb(
        tester,
        minRounds: 15,
        maxRounds: 400,
        until: () =>
            cacheStorageOf(comp.cachedFrames(
                    frames: BigInt.one, scale: p.uiState.viewerScale)[0]) !=
                0 &&
            cacheStats().entries > base,
      );

      final before = cacheStats();
      comp.renderScope(
        frame: BigInt.zero,
        scale: p.uiState.viewerScale,
        kind: 1,
        colours: scopeColoursFor(LumitTheme.dark()),
      );
      await settleFrb(tester, minRounds: 15, maxRounds: 80);

      // `best_frame` serves the held frame without touching the hit/miss
      // counters (they describe cache lookups, and this is a reuse before the
      // lookup) — so the observable is that nothing new was made: a fresh
      // composite would have filed another entry and counted a miss.
      final after = cacheStats();
      expect(after.entries, before.entries,
          reason: 'the second trace did not composite the composition again');
      expect(after.misses, before.misses,
          reason: 'and never even asked the cache for a render');
    });

    testWidgets('the bar is drawn under the ruler', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1000, 500),
      ));
      await tester.pump();

      expect(find.byKey(const ValueKey('tl-cache-bar')), findsOneWidget);
      expect(tester.getSize(find.byType(TimelineCacheBar)).height,
          TimelineCacheBar.height,
          reason: 'a thin stripe, per the design language');
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

    /// **A full cache follows the playhead.** The fill used to stop for good
    /// the moment the cache was within one frame of its budget, so on a full
    /// cache moving the playhead banked nothing: the frames it wanted were new,
    /// and the ones in the way were far off and stale. What it keeps now is a
    /// window around the playhead — so the frames near where you *are* end up
    /// held and the ones near where you *were* are the ones evicted for them.
    /// **A frame held in memory climbs back onto the card on its own.**
    ///
    /// The rungs of the ladder used to be climbed at the moment a frame was
    /// wanted — inside the turn that had to produce it. The climb from memory is
    /// only an upload, but it was paid out of that frame's budget instead of out
    /// of the slack the idle fill and the ring exist to bank. It is done in
    /// advance now (`line_up_frame`), and this pins the visible consequence: a
    /// frame pushed off the card and held in memory comes *back* to the card
    /// without anything asking for it, and without being composited again.
    testWidgets('a frame held in memory is put back on the card on its own',
        (tester) async {
      final p = freshProject();
      final comp = _stampComp(p.state.project!, 'Scene');
      p.uiState.setSelectedComp(comp);

      // Room for two frames, so banking a few pushes the earlier ones off the
      // card and into memory — which is the state this test is about.
      const room = (160 * 90 * 4 + 64) * 2;
      setVramCacheBudget(bytes: BigInt.from(room));
      addTearDown(() => setVramCacheBudget(bytes: BigInt.from(512 << 20)));

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      // The storage half of each strip byte: these tests ask whether a frame
      // is held, which is the low nibble's question (the resolution tier is
      // in the high one).
      List<int> tiers() => comp
          .cachedFrames(frames: BigInt.from(40), scale: 1.0)
          .map(cacheStorageOf)
          .toList();

      // Let the fill work around frame 0 until the frames near it are banked
      // and the ones behind have been pushed down into memory.
      await tester.runAsync(() async {
        for (var i = 0; i < 80; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          if (tiers()[1] == 2 && tiers()[2] == 2) return;
        }
        fail('the fill never warmed the frames around frame 0');
      });

      // Everything the fill has touched reads as held — on the card or one
      // upload away, which is what tier 2 means. The point is that it STAYS
      // that way while the fill carries on displacing things: a frame that
      // falls to memory is climbed back up rather than re-rendered, so the bar
      // never goes backwards.
      final held = tiers().where((t) => t == 2).length;
      await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 600)));
      expect(
        tiers().where((t) => t == 2).length,
        greaterThanOrEqualTo(held),
        reason: 'frames that fell to memory were climbed back, not lost',
      );
      expect(cacheStats().entries.toInt(), greaterThan(0),
          reason: 'and memory is holding the ones that left the card');
    });

    testWidgets('the fill follows the playhead even when the cache is full',
        (tester) async {
      final p = freshProject();
      final comp = _stampComp(p.state.project!, 'Scene');
      p.uiState.setSelectedComp(comp);

      // Room for four of this comp's frames and no more (160×90×4 bytes each,
      // plus a little bookkeeping), so the cache is full long before the comp
      // is and the window has to displace something to move.
      const room = (160 * 90 * 4 + 64) * 4;
      setVramCacheBudget(bytes: BigInt.from(room));
      // Back to the engine's own default afterwards: the budget is process-wide
      // and the tests after this one want room to work in.
      addTearDown(() => setVramCacheBudget(bytes: BigInt.from(512 << 20)));

      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      // Fill it around frame 0 first — the cache has to be genuinely full for
      // this to be a test of anything.
      List<int> tiers() => comp
          .cachedFrames(frames: BigInt.from(1000), scale: 1.0)
          .map(cacheStorageOf)
          .toList();
      await tester.runAsync(() async {
        for (var i = 0; i < 60; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          if (tiers()[1] == 2 && tiers()[2] == 2) return;
        }
        fail('the fill never warmed the frames around frame 0');
      });

      // Now go somewhere else entirely and leave the engine idle again.
      p.uiState.playheadFrame.value = 900;
      await tester.pump();
      await tester.runAsync(() async {
        for (var i = 0; i < 60; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          // A neighbour of the new position, not the frame itself: showing one
          // caches it, so only a neighbour proves the *fill* ran.
          if (tiers()[901] == 2) return;
        }
        fail('a full cache banked nothing around the new playhead');
      });

      // And what made room is the far side — but "made room" no longer means
      // "lost". **This is the demotion ladder end to end** (docs/06 §5.1, §5.3):
      // each frame evicted from the card is read back into memory and parked on
      // disk, so the old neighbourhood is still held — green, because a frame in
      // memory is one upload from the screen rather than a re-composite. Before
      // the ladder these frames were simply dropped and the two tiers below the
      // card were bookkeeping with nothing in them.
      final held = tiers();
      expect(held[1], 2, reason: 'evicted from the card, not thrown away');
      expect(held[2], 2);
      expect(cacheStats().entries.toInt(), greaterThan(0),
          reason: 'the frames that left the card were read back into memory');
      // The disk side of the same hand-off is proven in Rust
      // (`diskio::tests`, and `headless::tests` for the read-back), not here:
      // the disk numbers this process can see belong to whichever render worker
      // published last, and a test file keeps one worker per project alive, so
      // asserting on them here would be asserting on which worker spoke most
      // recently.
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
