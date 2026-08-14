# Dart and Flutter, taught from Lumit's code

For a developer who built VCL/FMX interfaces: component trees, properties, events.
Flutter is close enough to feel familiar and different enough to trip you twice.
Every example is real code from `flutter_ui/`.

## The two differences that matter

**1. Widgets are not components.** A VCL control is a long-lived object you mutate
(`Button1.Caption := 'x'`). A Flutter widget is an immutable *description*, rebuilt
constantly and cheaply. State lives outside the widget, in a `State` object or a
notifier. You never "set a property on a widget"; you change state and let the
description be rebuilt.

**2. Layout is a tree of nested widgets, not anchors.** There is no form designer and
no `Align`/`Anchors`. Padding is a widget. Centring is a widget. A row is a widget.
The nesting looks alarming at first and becomes readable quickly.

| Delphi | Flutter |
|---|---|
| `TForm` | The root widget of a route |
| `TPanel`, `TLayout` | `Container`, `Column`, `Row`, `Stack` |
| Control properties | Constructor arguments (immutable) |
| `OnClick` | `onPressed:` callback argument |
| `TComponent.Create(Owner)` lifetime | `initState` / `dispose` on a `State` |
| `TPaintBox.OnPaint` | `CustomPainter.paint(Canvas, Size)` |
| `TTimer` | `Timer`, `AnimationController`, `Ticker` |
| Observer/`TNotifyEvent` lists | `ChangeNotifier`, `ValueNotifier`, `Stream` |
| Global `Application.MainForm` | Provider / `InheritedWidget` lookup by context |

## Dart in five minutes

Dart is Java-shaped with type inference and null safety.

- `final` = assign once (use it by default). `var` = inferred, reassignable.
  `const` = compile-time constant.
- **Null safety**: `String` cannot be null; `String?` can. `?.` calls only when
  non-null; `??` supplies a default; `!` asserts non-null (and throws if wrong);
  `late` means "non-null, assigned before first use".
- `async`/`await` with `Future<T>`; `Stream<T>` for many values over time.
- Arrow bodies: `int double(int x) => x * 2;`
- Named arguments are the norm: `Text('hi', style: s)`. `required` marks the
  mandatory ones.
- **Records** (Dart 3): `(CompositionReference, String)` — a tuple with types.
- **Patterns** (Dart 3): destructuring in `switch` and `case`.

## 1. Bootstrap

```dart
// flutter_ui/lib/main.dart:207
Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  // Sweep up after an update before anything else happens (K-297): delete the
  // version we have just replaced, now that nothing is holding its files, and
  // put it back if a swap was cut in half. Never throws and never blocks — a
  // tidying problem is not a reason for an editor not to open.
  tidyAfterUpdate(InstallSite.detect());

  // The call tracer takes StackTrace.current on every bridge call, which is
  // debugging money a release build must not spend.
  await BridgeLib.init(handler: kDebugMode ? CustomHandler() : null);
```

`main` is `async` because loading the Rust library is a `Future`. `runApp` at the end
hands the root widget to the framework.

## 2. The root: providers and rebuild scope

```dart
// flutter_ui/lib/main.dart:1841
      home: ChangeNotifierProvider.value(
        value: state,
        child: ChangeNotifierProvider.value(
          value: uiState,
          // Rebuilt when the workspace changes, so the scale slider and the
          // scheme picker take effect as they are moved.
          child: ListenableBuilder(
            listenable: uiState,
            builder: (context, _) => ThemeScope(
              theme: uiState.theme,
              animationLevel: uiState.workspace.animationLevel,
              showTooltips: uiState.workspace.interface.showTooltips,
```

Read this as: make two objects available to everything below (`Provider`), and
rebuild this subtree whenever `uiState` notifies (`ListenableBuilder`). The `child:`
argument nesting *is* the tree.

## 3. State objects and lifecycle

A `StatefulWidget` has a `State` with `initState` and `dispose` — the Delphi
constructor/destructor pair:

```dart
// flutter_ui/lib/main.dart:1936
  @override
  void initState() {
    super.initState();
    // Shortcuts are handled GLOBALLY, not through the focus tree. Every menu,
    // popup and palette lives in the Overlay outside this view's scope, so
    // any of them could walk focus away and never bring it back — and every
    // shortcut died until something was clicked (the space bar's recurring
    // funeral). A hardware-keyboard handler fires wherever focus is; the
    // focused-text-field guard inside _onKey keeps typing safe.
    HardwareKeyboard.instance.addHandler(_handleKey);
```

Whatever you register in `initState` you remove in `dispose`. Tests fail on leaked
handlers and timers.

## 4. Notifiers: the observer pattern, built in

`ValueNotifier<T>` holds one value and notifies listeners on change:

```dart
// flutter_ui/lib/main.dart:254
  /// The status bar's one-line notice: the latest quiet message or genuine
  /// error, dismissed by its close button. One current notice rather than a
  /// feed, which is what the egui shell's `app.notice` was too.
  final ValueNotifier<LumitNotice?> notice = ValueNotifier(null);

  void postNotice(String message, {bool error = false}) =>
      notice.value = LumitNotice(message, error: error);
```

Widgets listen with `ValueListenableBuilder`, which rebuilds **only** its builder —
the key performance tool. Note `child:` here: a subtree that does not depend on the
value is built once and passed in:

```dart
// flutter_ui/lib/panels/timeline_panel_frb.dart:366
    return ValueListenableBuilder<LayerDrag?>(
      valueListenable: drag,
      child: child,
      builder: (context, value, child) {
        final height = index < heights.length ? heights[index] : 0.0;
        return AnimatedSlide(
          offset: height <= 0
              ? Offset.zero
              : Offset(0, layerDragShift(heights, value, index) / height),
          duration: duration,
          curve: Curves.easeOut,
          child: child,
```

`Listenable.merge` subscribes to several at once:

```dart
// flutter_ui/lib/panels/viewer_panel_frb.dart:794
                listenable: Listenable.merge([
                  uiState.selectedLayers,
                  uiState.layerBounds,
                  uiState.model,
                  uiState.liveRotations,
                  uiState.liveText,
                  uiState.liveTransforms,
                ]),
                builder: (context, _) {
                  final boxes = _boxes();
```

## 5. Talking to Rust

Calls are ordinary methods on generated handles. `async` ones are awaited; failures
arrive as exceptions:

```dart
// flutter_ui/lib/state/keymap.dart:275
  Future<String?> rebind(
      BridgeKeyContext context, String action, String chord) async {
    try {
      _adopt(await keymapRebind(
          context: context, action: action, chord: chord));
      return null;
    } on AnyhowException catch (e) {
      return e.message;
    }
  }
```

Events arrive as a `Stream`. Dart 3 pattern matching over the generated sealed
classes makes the handler exhaustive and readable:

```dart
// flutter_ui/lib/main.dart:1381
    sub = state.onWorkerResponse.listen((msg) {
      switch (msg) {
        case WorkerResponse_RenderedDMABuf frame:
          previewTier.value = frame.field0.tier;
          _showDmabuf(frame.field0);
        case WorkerResponse_RenderedSharedTexture frame:
          previewTier.value = frame.field0.tier;
          _showSharedTexture(frame.field0);
        // Scope traces ride the same stream; the Scopes panel subscribes to it
        // directly, so there is nothing for the Viewer to do with one.
        case WorkerResponse_Scope():
          break;
```

Records and destructuring patterns, with a nullable cache:

```dart
// flutter_ui/lib/main.dart:414
  List<(CompositionReference, String)>? _compsCache;
  List<(CompositionReference, String)> comps() {
    if (_compsCache != null) return _compsCache!;
    final out = <(CompositionReference, String)>[];
    void walk(List<ItemReference> items) {
      for (final item in items) {
        switch (item) {
          case ItemReference_Composition(:final field0):
            out.add((field0, field0.getSettings().name));
          case ItemReference_Folder(:final field0):
            walk(field0.getChildren());
```

`case ItemReference_Composition(:final field0)` binds the field and names it in one
step. Note also the nested function `walk` — Dart allows local functions, like
Delphi's nested procedures.

## 6. Theme through an InheritedWidget

`InheritedWidget` propagates data down the tree and rebuilds dependents when it
changes. Lookup is by type from a `BuildContext`:

```dart
// flutter_ui/lib/widgets/controls.dart:70
  static ThemeScope of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<ThemeScope>()!;

  @override
  bool updateShouldNotify(ThemeScope old) =>
      old.theme != theme ||
      old.animationLevel != animationLevel ||
      old.showTooltips != showTooltips;
```

Widgets read semantic tokens, never a literal colour:

```dart
// flutter_ui/lib/widgets/controls.dart:128
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final enabled = widget.onPressed != null;
    Color? fill;
    Color? edge;
    if (!enabled) {
      fill = widget.frameless ? null : t.surface2;
    } else if (_down) {
      fill = t.hairlineStrong;
      edge = t.accent;
    } else if (_hover) {
```

A hex literal outside `lib/theme/` fails CI.

## 7. Custom painting

This is `TPaintBox.OnPaint` with a better API. A `CustomPainter` gets a `Canvas` and
a `Size`:

```dart
// flutter_ui/lib/panels/timeline_panel_frb.dart:7324
    const half = 4.0;
    final mid = size.height / 2;
    for (var i = 0; i < frames.length; i++) {
      final x = axis.xOf(frames[i]);
      canvas.drawPath(
        Path()
          ..moveTo(x, mid - half)
          ..lineTo(x + half, mid)
          ..lineTo(x, mid + half)
          ..lineTo(x - half, mid)
          ..close(),
        Paint()..color = selected.contains(i) ? chosen : colour,
```

`..` is the cascade operator: call several methods on one object and keep the object.
It replaces Delphi's `with` block, safely.

Painters are testable with a recording canvas — no window required:

```dart
// flutter_ui/test/waveform_test.dart:377
class _RecordingCanvas implements Canvas {
  final List<_Stroke> lines = [];

  @override
  void drawLine(Offset p1, Offset p2, Paint paint) =>
      lines.add(_Stroke(p1, p2, paint.color));

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}
```

`noSuchMethod` absorbs the rest of the interface, so the test implements only what it
checks.

## 8. Gestures and coordinates

Pointer positions arrive in local widget coordinates. Converting to scene coordinates
is your maths, kept pure and testable:

```dart
// flutter_ui/lib/panels/viewer_layer_map.dart:171
  Offset toScreen(double x, double y) {
    final dx = (x - ax) * sx;
    final dy = (y - ay) * sy;
    final rx = dx * cos - dy * sin;
    final ry = dx * sin + dy * cos;
    return Offset(
      origin.dx + (px + rx) * viewScale,
      origin.dy + (py + ry) * viewScale,
    );
  }
```

Hit-testing runs the inverse map, so it stays exact under rotation:

```dart
// flutter_ui/lib/panels/viewer_gizmo.dart:249
  bool contains(Offset point) {
    final p = map.layerOf(point);
    return p.dx >= 0 &&
        p.dy >= 0 &&
        p.dx <= bounds.width &&
        p.dy <= bounds.height;
  }
```

Drags accumulate pixels and derive frames from the running total. Snapping measures
candidates in screen pixels, so zoom controls precision:

```dart
// flutter_ui/lib/panels/timeline_snap.dart:107
  SnapTarget? best;
  var bestPx = slopPx;
  for (final target in targets) {
    final px = ((target.frame - frame) * perFrame).abs();
    // Strictly nearer, so the first of two equally close targets keeps it and
    // the answer does not depend on the order a caller happened to gather them.
    if (px < bestPx) {
      bestPx = px;
      best = target;
    }
  }
  if (best != null) return (frame: best.frame, caught: best);
```

That returns a **named record** — `(frame: ..., caught: ...)`.

## 9. Layout that responds to its box

`LayoutBuilder` gives you the constraints your parent offers:

```dart
// flutter_ui/lib/panels/viewer_panel_frb.dart:357
    final stage = Expanded(
      child: LayoutBuilder(
        builder: (context, constraints) {
          final size = facts.size;
          final fitted = _fittedRect(constraints, size);
          _reportScale(ui, fitted, size, _fitScale(constraints, size));
```

## 10. The Viewer's engine frame

The engine's picture is a platform texture, not an image. Flutter draws it with the
`Texture` widget given a texture id:

```dart
// flutter_ui/lib/panels/viewer_panel_frb.dart:1363
        final picture = textureId != null
            ? Texture(
                textureId: textureId,
                filterQuality: uiState.workspace.smoothZoomedViewer
                    ? FilterQuality.low
                    : FilterQuality.none,
              )
            : const SizedBox.expand();
        return pictureChannelFilter(channel, picture);
```

## 11. Keyboard

Lumit handles shortcuts globally rather than through the focus tree, and stands down
when a text field has focus or a modal is open:

```dart
// flutter_ui/lib/panels/timeline_panel_frb.dart:1566
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted) return false;
    // A dialogue is up: its keys are its own (K-243). These commands are
    // registered on the hardware keyboard rather than on focus, so without
    // this the Pre-compose dialogue's `Enter` also renamed the layer behind it.
    if (lumitModalOpen) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
```

Dispatch is a plain switch over action ids resolved from the engine's keymap:

```dart
// flutter_ui/lib/main.dart:2107
    var handled = true;
    switch (action) {
      case 'edit.redo':
        project?.redo();
        state.notifyDocumentChanged();
      case 'edit.undo':
        project?.undo();
        state.notifyDocumentChanged();
      case 'playback.toggle':
        ui.requestTogglePlay();
```

Dart 3 `switch` statements do not fall through, so there is no `break` noise.

## 12. Dialogs

Lumit does not use Material dialogs. `showLumitModal` builds an overlay entry and
returns a `Future` that completes with the result:

```dart
// flutter_ui/lib/widgets/controls.dart:969
Future<T?> showLumitModal<T>({
  required BuildContext context,
  required Widget Function(void Function(T?) close) builder,
  String? id,
  Size? initialSize,
  Size minSize = const Size(320, 240),
}) {
  final overlay = Overlay.of(context);
  final completer = Completer<T?>();
  late OverlayEntry entry;
  void close(T? v) {
    if (completer.isCompleted) return;
```

`Completer` is the manual way to produce a `Future`. `late` here means "assigned
before use" — needed because `entry` and `close` reference each other.

## 13. Tests

Pure logic tests need no widgets at all:

```dart
// flutter_ui/test/window_title_test.dart:7
void main() {
  test('no path is plain Lumit', () {
    expect(windowTitleFor(null), 'Lumit');
    expect(windowTitleFor(''), 'Lumit');
  });

  test('a Windows path shows the file name without .lum', () {
    expect(windowTitleFor(r'C:\work\Shot 01.lum'), 'Lumit - Shot 01');
  });
}
```

`r'...'` is a raw string — no escape processing, ideal for Windows paths.

Widget tests pump a widget and assert on the tree; the `test/frb/` suites drive the
**real** Rust engine, which is why they run serially.

## Reading order in this repo

1. `lib/state/timecode.dart` — pure Dart, no Flutter.
2. `lib/panels/timeline_snap.dart` — pure maths with records.
3. `lib/widgets/controls.dart` — the house widgets; see how tokens and state combine.
4. `lib/panels/scopes_panel_frb.dart` — a complete, medium-sized panel.
5. `lib/main.dart` — read last; it is the wiring for everything else.

## The house rules that will surprise you

- **Zero bridge calls in `build` or `paint`** (K-184). Read `CompModel`.
- Every string goes through `app_en.arb` and is read as `l10n.someKey`.
- No colour literal outside `lib/theme/`.
- `lib/src/rust/**` and `lib/l10n/gen/**` are generated. Edit the source, regenerate.
- Give conditional `Stack` children near a gesture a `ValueKey`, or drags break
  mid-gesture.
- Decoration painters need `hitTest => false`, or they swallow gestures beneath them.
