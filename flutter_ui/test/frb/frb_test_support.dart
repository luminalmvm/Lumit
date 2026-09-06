// The harness every frb panel test shares.
//
// **Why these are integration tests, not fake-bridge unit tests.** The v0 panels
// took a `DocumentBridge` interface, so a test could hand them a fake. The frb
// generated types are concrete classes that call straight into the native library,
// so there is nothing to substitute — and adding a Dart interface over them purely
// to allow faking would reintroduce exactly the mirror-class indirection the
// migration exists to delete.
//
// So these tests drive the real engine: `flutter test` loads the built
// `lumit_bridge` library and every document operation is the genuine one. That is
// strictly better coverage than a fake, which can only ever assert that Dart
// *called* something — a fake cannot tell you the op did what you meant, and it
// drifts silently when the engine changes. The cost is a build dependency: the
// library must exist and be in sync, or frb refuses to start (it compares a
// content hash on both sides).
//
// Run `cargo build -p lumit_bridge` first, and re-run it after any change to
// `crates/lumit-bridge/src/api/**` — a stale library fails loudly with a content
// hash mismatch rather than misbehaving quietly.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:provider/provider.dart';

/// Where `cargo build -p lumit_bridge` leaves the library, relative to
/// `flutter_ui/`. The cargokit-built copy under `build/` is not used: it only
/// exists after a full `flutter build`, which a test run should not require.
String get _libraryPath {
  final stem = Platform.isWindows
      ? 'lumit_bridge.dll'
      : Platform.isMacOS
          ? 'liblumit_bridge.dylib'
          : 'liblumit_bridge.so';
  return '../target/debug/$stem';
}

bool _initialised = false;

/// Load the engine once per test process.
///
/// Skips the whole group with a clear instruction when the library is absent,
/// rather than failing with an opaque FFI error — a contributor who has not built
/// the Rust side should be told what to run.
Future<void> initEngineForTests() async {
  if (_initialised) return;
  final library = File(_libraryPath);
  if (!library.existsSync()) {
    throw StateError(
      'The engine library is not built. Run:\n'
      '  cargo build -p lumit_bridge\n'
      'Looked for: ${library.absolute.path}',
    );
  }
  await BridgeLib.init(externalLibrary: ExternalLibrary.open(_libraryPath));
  // A test must never reach the developer's own settings file. Any setter on
  // a Workspace calls `save()`, so without this redirect a run wrote defaults
  // straight over `%APPDATA%\lumit\flutter-workspace.json` — which is exactly
  // why settings kept resetting between builds.
  Workspace.storeOverride =
      '${Directory.systemTemp.createTempSync('lumit-ws').path}/workspace.json';
  _initialised = true;
}

/// True when the engine library is present, for `skip:` on a whole group.
bool get engineAvailable => File(_libraryPath).existsSync();

/// True when this machine cannot hand the Viewer a frame at all.
///
/// Zero-copy is the only Viewer transport: a frame reaches Dart as a
/// platform texture handle or it does not reach Dart. On a machine whose Vulkan
/// driver cannot export the shared image — Mesa's lavapipe, the software
/// rasteriser the Linux CI runner has instead of a GPU, where
/// `vkAllocateMemory` refuses the exportable allocation — every frame is
/// dropped at the publish step. The engine says so on stderr and carries on;
/// nothing crashes, and nothing arrives.
///
/// So the tests that wait for a frame cannot pass there, and would not be
/// telling the truth if they did. They skip on this flag rather than being
/// deleted or loosened, because on a machine with a real adapter — the owner's
/// Windows box, any developer's — they still run and still fail on a genuine
/// regression. Set `LUMIT_NO_ZERO_COPY_VIEWER=1` to declare a machine
/// transportless; CI sets it for the Linux job (.github/workflows/ci.yml) and
/// nothing else does.
///
/// This is a hole in the gate and worth closing: the Linux DMA-BUF path
/// (docs/TODO.md) has still never run against hardware, and until a Linux
/// machine with a real GPU runs these, no CI job exercises frame delivery on
/// any platform.
bool get zeroCopyViewerUnavailable =>
    Platform.environment['LUMIT_NO_ZERO_COPY_VIEWER'] == '1';

/// Mount [child] with the providers and theme scope a panel needs.
///
/// The `Overlay` is load-bearing: the project context menu and every dialog are
/// overlay entries. `showTooltips: false` keeps `LumitTooltip` out of the widget
/// tree so text finders are not confused by tooltip copy.
Widget hostPanel({
  required Widget child,
  required LumitState state,
  required LumitUiState uiState,
  Size size = const Size(480, 760),

  /// How much motion the panel under test is allowed. None by default, so a
  /// test asserts a finished state rather than racing an animation; a test
  /// that is *about* the motion (the Viewer's zoom flight) asks for it.
  AnimationLevel animationLevel = AnimationLevel.none,

  /// Which shape's chrome the panel is dressed in. Sharp by default, because
  /// that is what every behaviour test wants to assert against; a test about
  /// Round's own geometry asks for it.
  ThemeShape shape = ThemeShape.sharp,

  /// How much room a row gets. Regular by default, because that is
  /// what the editor ships as and what every mockup renders; a test about the
  /// **Compact** setting passes `DensityTokens.compact` and asserts the
  /// tighter column of §12A.6's table.
  DensityTokens density = DensityTokens.regular,
}) =>
    Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: MediaQueryData(size: size),
        child: ChangeNotifierProvider<LumitState>.value(
          value: state,
          child: ChangeNotifierProvider<LumitUiState>.value(
            value: uiState,
            child: ThemeScope(
              theme: LumitTheme.forScheme(LumitColorScheme.dark, shape)
                  .copyWith(density: density),
              animationLevel: animationLevel,
              showTooltips: false,
              // The application's root is a MaterialApp, which puts one of
              // these above everything; without it `onTapOutside` never fires
              // and a test cannot see an inline editor commit on a click
              // elsewhere.
              // For the same reason: a MaterialApp puts WidgetsLocalizations
              // above everything, and the widgets library's own reorderable
              // list asks for them by name (the export queue's draggable
              // rows). Without this the panel builds and the list throws.
              child: Localizations(
                locale: const Locale('en'),
                delegates: const [DefaultWidgetsLocalizations.delegate],
                child: TapRegionSurface(
                  child: _StopsPreviewProgress(
                    uiState: uiState,
                    child: Overlay(
                      initialEntries: [OverlayEntry(builder: (_) => child)],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );

/// Stops the preview-progress timer when the tree comes down.
///
/// `addTearDown(uiState.dispose)` below cancels that timer too, but it runs too
/// late to help the test that started it: `flutter_test` unmounts the tree,
/// pumps, and *then* asserts that no timer is pending — all before a single
/// `addTearDown` callback is called. So a test whose last render report lands
/// within the tracker's 150 ms delay ends with a timer it cannot cancel, and
/// fails on a bar that was never going to be drawn.
///
/// A widget's `dispose` runs during that unmount, which is early enough. Every
/// frb test mounts through [hostPanel], so this covers all of them rather than
/// each test having to remember to wait for `previewProgress.idle`.
class _StopsPreviewProgress extends StatefulWidget {
  const _StopsPreviewProgress({required this.uiState, required this.child});

  final LumitUiState uiState;
  final Widget child;

  @override
  State<_StopsPreviewProgress> createState() => _StopsPreviewProgressState();
}

class _StopsPreviewProgressState extends State<_StopsPreviewProgress> {
  @override
  void dispose() {
    widget.uiState.previewProgress.stop();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.child;
}

/// A fresh engine-backed project and its UI state.
///
/// Each call makes a new project with its own id, so tests do not collide in the
/// engine's process-wide registry — but no test may call `openProject`, which
/// clears that registry wholesale.
({LumitState state, LumitUiState uiState}) freshProject() {
  final state = LumitState()..newProject();
  // A default workspace, deliberately NOT loaded from disk: `Workspace()..load()`
  // reads the developer's own settings file, so a test would assert against
  // whatever colour scheme the machine happened to be set to.
  final uiState = LumitUiState(state, workspace: Workspace());
  // Known to [settleFrb], which ends every settle with this state's preview
  // tracker idle — see [_settleProgress] for the failure that needs it.
  _liveUis.add(uiState);
  addTearDown(() => _liveUis.remove(uiState));
  // Every one of these listens to the engine's response stream and holds the
  // preview-progress timer. Dropped on the floor at the end of a test they do
  // not stop listening, so by the end of a file a dozen dead UI states are
  // still taking reports — and a timer one of them starts fires inside some
  // later, unrelated test, which then fails on a pending timer it never
  // created. That is precisely how `cache_bar_frb_test` went red on main while
  // passing everywhere else: it was not its timer.
  addTearDown(uiState.dispose);
  // Close the engine project when the test is over. Each one left open keeps
  // its render worker — and that worker's whole GPU device — alive for the
  // rest of the test process, and a file with fifty tests piled up fifty
  // renderers: on the Linux CI runner's software Vulkan those all live in
  // ordinary memory, which is how the runner ran out of it mid-suite.
  // Teardowns run last-registered-first, so this runs before uiState.dispose;
  // the worker's response stream simply ends, which every listener survives.
  addTearDown(() => state.project?.close());
  return (state: state, uiState: uiState);
}

/// Let an async frb call actually finish inside a `testWidgets` body.
///
/// **Why this exists — the fake-async/real-async seam.** `testWidgets` runs its
/// body inside a `FakeAsync` zone: microtasks and timers are queued, and the
/// whole body is driven by `fakeAsync.flushMicrotasks()` from
/// `AutomatedTestWidgetsFlutterBinding.runTest`. The isolate therefore never
/// returns to the *real* event loop. An async frb call finishes by way of a
/// native `ReceivePort` message (`BaseHandler.executeNormal` completes a
/// `Completer` from that port), and a port message is only delivered on a real
/// event-loop turn — so on its own a `pump` can never complete one.
///
/// `tester.runAsync` supplies the real turns, but it is only half the fix,
/// because the *continuation* is not in the same zone as the delivery:
///
/// * The panel's `getStatus().then(…)` is registered during `build`, i.e. in the
///   fake zone. The port message arriving during `runAsync` completes the
///   completer synchronously (frb's port uses a sync `StreamController`), but the
///   `.then` after it is queued in **FakeAsync's** microtask queue. Nothing
///   drains that queue until the body is back in the fake zone.
/// * A bare `pump()` is not enough to drain it: it only flushes microtasks
///   `if (hasScheduledFrame)`, and even then it flushes *before* the frame — so
///   the `setState` that the flush triggers is drawn a frame late. `pump(Duration
///   .zero)` goes through `FakeAsync.elapse`, which always flushes microtasks
///   first, so the same pump draws the result.
/// * Never `await` a panel's frb future from inside `runAsync`: `runAsync` cannot
///   return until its callback does, and `runTest` only flushes FakeAsync once
///   `runAsync` has returned — so a future whose continuation sits in the fake
///   queue deadlocks the test outright rather than failing. (A future *created*
///   inside `runAsync` is fine; its continuations are real.)
///
/// Hence the loop: one real turn, one fake flush, repeat. [minRounds] rounds
/// always run, because `LumitState` also carries the engine's `ScopedChange` and
/// worker streams — their backlog is delivered on those same real turns, and a
/// `ScopedChange` makes a panel discard cached async results and ask again. So
/// the first answer is not necessarily the last one, and settling has to be
/// iterative rather than a single sleep.
///
/// [until] stops the loop early once the thing under test has appeared; the
/// caller still asserts it, so this returns quietly on exhaustion rather than
/// failing with a message that would only duplicate the caller's `reason`.
///
/// The same deadlock catches asynchronous `dart:io` — `await
/// Directory.systemTemp.createTemp(…)` in a `testWidgets` body hangs the test
/// rather than failing it. Use the `…Sync` variants in a widget test.
///
/// Note also that this deliberately elapses **no fake time**. Anything waiting on
/// a fake timer — an animation, or a `DoubleTapGestureRecognizer` holding the
/// gesture arena for `kDoubleTapTimeout` — still needs the test's own
/// `pump(duration)` or `pumpAndSettle`.
Future<void> settleFrb(
  WidgetTester tester, {
  bool Function()? until,
  Duration slice = const Duration(milliseconds: 20),
  int minRounds = 4,
  int maxRounds = 40,
}) async {
  for (var round = 0; round < maxRounds; round++) {
    // A real event-loop turn: frb port messages land here.
    await tester.runAsync(() => Future<void>.delayed(slice));
    // Back in fake async: flush the continuations those messages queued, and
    // draw the frame their `setState`s asked for.
    await tester.pump(Duration.zero);
    if (round + 1 >= minRounds && (until == null || until())) break;
  }
  await _settleProgress(tester, slice);
}

/// The UI states [freshProject] has made and not yet torn down.
final List<LumitUiState> _liveUis = [];

/// End a settle with no preview-progress timer pending.
///
/// **The failure this exists for.** The engine reports how far a waited-on
/// frame has got, and the tracker arms a 150 ms timer on the first report to
/// decide whether a bar is worth drawing. That report is delivered inside the
/// fake-async pump above, so the timer is a *fake* timer — and `testWidgets`
/// fails a body that returns with one pending, checking **before** the
/// teardown that would have disposed it. So any test whose last settle
/// coincides with a slow frame's first report failed on a timer it never
/// created: `cache_bar_frb_test` on main, then `group_effects_frb_test` on
/// the Linux runner, whose software renderer is exactly what makes a frame
/// slow enough to report. A round count cannot fix that; the tracker's own
/// `idle` can.
///
/// A few more real turns for the frame to finish, then — if it has not — the
/// tracker is stopped, which cancels the timer. Stopping forgets a bar nobody
/// in a test was drawing; the render itself carries on and lands as it would.
Future<void> _settleProgress(WidgetTester tester, Duration slice) async {
  for (var round = 0; round < 25; round++) {
    if (_liveUis.every((ui) => ui.previewProgress.idle)) return;
    await tester.runAsync(() => Future<void>.delayed(slice));
    await tester.pump(Duration.zero);
  }
  for (final ui in _liveUis) {
    if (!ui.previewProgress.idle) ui.previewProgress.stop();
  }
}

/// The [settleFrb] ceiling to allow the **first** picture of a worker session.
///
/// A worker builds its renderer before it reads its first request — a GPU
/// device, then every pipeline the compositor needs — and it does that once per
/// project, so every frb test that mounts a fresh project pays it again. Where
/// the driver has no warm shader cache it is seconds rather than milliseconds:
/// measured on a Windows development machine at 3.3–5.0 s, against a first
/// render of about 30 ms once the renderer stands. A ceiling in the low
/// hundreds of milliseconds therefore cannot be met however healthy the engine
/// is, and the tests that wait for a picture fail on the machine rather than on
/// the code — which is exactly how they read. The build may also queue behind
/// one other worker's, which is what stops a file of them exhausting
/// the card — so the ceiling covers a turn as well as a build.
///
/// Ten seconds of ceiling costs nothing where the frame is quick: [settleFrb]
/// returns the moment its `until` is true, so a warm machine still finishes in
/// a few rounds. Only use it for a wait that includes a cold worker's first
/// frame; a wait for a *second* frame is a real render and wants a real
/// ceiling, so that a stall in one still fails.
const int coldWorkerRounds = 500;

/// A second tap, far enough after the first to read as two singles rather than a
/// double-tap — the click-then-click-again rename gesture.
Future<void> tapAgain(WidgetTester tester, Finder target) async {
  await tester.tap(target);
  await tester.pump(const Duration(milliseconds: 350));
  await tester.tap(target);
  await tester.pump(const Duration(milliseconds: 350));
}

/// The number a **still** scalar holds — for asserting on a value that a test
/// never keyed. A keyed one has no single number, so asking for it here is a
/// test bug rather than a value of zero, and it says so.
double stillValue(BridgeScalar scalar) => switch (scalar) {
      BridgeScalar_Static(:final field0) => field0,
      _ => throw StateError('expected a still scalar, got $scalar'),
    };

/// The Viewer bottom bar's controls, left to right, by key.
///
/// The bar's arrangement is a decision rather than a look, so what asserts it
/// is the order of the keys and not a picture. Only the keys that name a
/// control are collected — a slot standing empty has nothing to name, and the
/// wrappers between them are not the point. Element traversal is depth-first
/// from the bar's own `ValueKey`, which for a `Row` is left to right.
List<String> barKeys(WidgetTester tester) => _keysUnder(tester, 'viewer-bar');

/// The same for the Viewer's **header** strip: the three pickers the drawing
/// puts at its right-hand end.
List<String> headerKeys(WidgetTester tester) =>
    _keysUnder(tester, 'viewer-header');

List<String> _keysUnder(WidgetTester tester, String key) => [
      for (final element in find
          .descendant(
            of: find.byKey(ValueKey<String>(key)),
            matching: find.byWidgetPredicate((w) => w.key is ValueKey<String>),
          )
          .evaluate())
        (element.widget.key! as ValueKey<String>).value,
    ];
